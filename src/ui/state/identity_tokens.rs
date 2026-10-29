//! Identity Hub Tokens tab state: the selected identity's token rows, the
//! balance requests it waits on, and the list controls remembered per device.
//!
//! Renders nothing. Rows are derived from the token registry, the per-pair
//! dismissals and the upstream balance snapshot, all read through
//! `AppContext`; the derivation is cached for a few seconds because the
//! snapshot changes without telling the UI, and shared by `Arc` so a frame
//! never clones it.

use crate::app::AppAction;
use crate::backend_task::error::TaskError;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::token::{
    IdentityTokenBalance, IdentityTokenBasicInfo, IdentityTokenIdentifier,
    IdentityTokenMaybeBalanceWithActions, IdentityTokensViewPrefs, TokenListSort,
    get_available_token_actions_for_identity, once_per_identity_amount, token_display_name,
};
use crate::ui::components::confirmation_dialog::ConfirmationDialog;
use crate::ui::state::PendingRequest;
use crate::ui::state::pending_request::PendingRequests;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v1::TokenConfigurationV1Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use std::borrow::Borrow;
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A balance refresh is due on arrival when the last one is older than this.
pub const ARRIVAL_REFRESH_AFTER: Duration = Duration::from_secs(2 * 60);

/// Derived rows are re-read from the snapshot after this long.
const ROWS_TTL: Duration = Duration::from_secs(5);

/// The filter and sort controls appear from this many rows.
pub const MANY_TOKENS_THRESHOLD: usize = 8;

/// A once-per-identity distribution as it concerns the selected identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnceClaim {
    /// What every identity may claim once, in the token's smallest units.
    pub amount: TokenAmount,
    /// When this identity took its claim, as far as this device knows.
    pub claimed_at_ms: Option<u64>,
}

/// One tracked token of the selected identity: the shared model record plus
/// what only the hub shows.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenRow {
    /// The token, its display name (sanitized registry name, also the balance
    /// unit), its balance when known, and the actions computed without the
    /// developer display override.
    pub token: IdentityTokenMaybeBalanceWithActions,
    /// The contract owner, who issued the token.
    pub issuer_id: Identifier,
    /// The identity issued the token (owns its contract).
    pub issued_by_me: bool,
    pub once_claim: Option<OnceClaim>,
    /// The identity holds an administrative permission on the token.
    pub has_admin_permission: bool,
    /// No balance is known and the last check that would have found one failed.
    pub check_failed: bool,
}

impl TokenRow {
    pub fn pair(&self) -> IdentityTokenIdentifier {
        IdentityTokenIdentifier {
            identity_id: self.token.identity_id,
            token_id: self.token.token_id,
        }
    }

    /// The display name, also used as the balance unit.
    pub fn name(&self) -> &str {
        &self.token.token_alias
    }

    /// `None` until the identity's balance of this token has been synced.
    pub fn balance(&self) -> Option<TokenAmount> {
        self.token.balance
    }

    /// Holders may send this token to another identity.
    pub fn transferable(&self) -> bool {
        self.token.token_config.is_transferable()
    }

    /// A distribution this identity may claim, before once-per-identity bookkeeping.
    pub fn can_claim(&self) -> bool {
        self.token.available_actions.can_claim
    }

    /// `value` in the token's decimals, with the name as unit.
    pub fn amount(&self, value: TokenAmount) -> Amount {
        self.token.amount_of(value)
    }

    /// The balance as the Send screen expects it, once known.
    pub fn balance_record(&self) -> Option<IdentityTokenBalance> {
        self.token
            .balance
            .map(|balance| self.token.to_token_balance(balance))
    }

    /// The handles the Claim screen expects.
    pub fn basic_info(&self) -> IdentityTokenBasicInfo {
        (&self.token).into()
    }

    /// The token ID as users copy and compare it.
    pub fn token_id_base58(&self) -> String {
        self.token.token_id.to_string(Encoding::Base58)
    }

    /// A claim the identity can still take: a distribution it may claim,
    /// unless its once-per-identity claim is already taken.
    pub fn claim_available(&self) -> bool {
        self.can_claim()
            && !self
                .once_claim
                .as_ref()
                .is_some_and(|once| once.claimed_at_ms.is_some())
    }

    /// The identity holds some of this token.
    pub fn holds_some(&self) -> bool {
        self.token.balance.is_some_and(|balance| balance > 0)
    }

    /// The local date this identity took its once-per-identity claim, if it did.
    pub fn claimed_on_date(&self) -> Option<String> {
        let claimed_at_ms = self.once_claim.as_ref()?.claimed_at_ms?;
        Some(
            crate::model::datetime::instant_from_unix_millis(claimed_at_ms)
                .map(crate::model::datetime::local_date)
                .unwrap_or_default(),
        )
    }

    /// Whether the name or ID contains `filter`, case-insensitively. An empty
    /// filter matches everything.
    pub fn matches_filter(&self, filter: &str) -> bool {
        let filter = filter.trim().to_lowercase();
        if filter.is_empty() {
            return true;
        }
        self.name().to_lowercase().contains(&filter)
            || self.token_id_base58().to_lowercase().contains(&filter)
    }
}

/// One status line per row; text first, colour only reinforces it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RowStatus {
    /// Nothing to say.
    None,
    /// The identity's balance of this token has not been synced yet.
    Checking,
    /// The last check that would have found a balance failed; none is known.
    CouldNotCheck,
    /// The identity has no signing key on this device.
    AddKey,
    /// A once-per-identity claim this identity may still take.
    CanClaimOnce(Amount),
    /// The once-per-identity claim was taken; the local date it happened.
    ClaimedOn(String),
    /// A perpetual or pre-programmed distribution this identity may claim.
    RewardsMayBeWaiting,
}

/// The status line for `row`. `view_only` says the identity cannot sign on
/// this device, which blocks Send and Claim alike.
pub fn row_status(row: &TokenRow, view_only: bool) -> RowStatus {
    if row.balance().is_none() {
        return if row.check_failed {
            RowStatus::CouldNotCheck
        } else {
            RowStatus::Checking
        };
    }
    if view_only && (row.claim_available() || (row.holds_some() && row.transferable())) {
        return RowStatus::AddKey;
    }
    if let Some(once) = &row.once_claim {
        return match row.claimed_on_date() {
            Some(date) => RowStatus::ClaimedOn(date),
            None => RowStatus::CanClaimOnce(row.amount(once.amount)),
        };
    }
    if row.claim_available() {
        return RowStatus::RewardsMayBeWaiting;
    }
    RowStatus::None
}

/// Order `rows` per `sort`. Zero balances always come last; unknown balances
/// sit between known ones and zeros, so a pending check never looks like a
/// zero. Ties break on name.
pub fn sort_rows<R: Borrow<TokenRow>>(rows: &mut [R], sort: TokenListSort) {
    rows.sort_by(|a, b| {
        let (a, b) = (a.borrow(), b.borrow());
        let by_name = || a.name().to_lowercase().cmp(&b.name().to_lowercase());
        let by_balance = || balance_rank(b).cmp(&balance_rank(a));
        match sort {
            TokenListSort::ClaimableFirst => b
                .claim_available()
                .cmp(&a.claim_available())
                .then_with(by_balance)
                .then_with(by_name),
            TokenListSort::BalanceHighToLow => by_balance().then_with(by_name),
            TokenListSort::NameAToZ => by_name(),
        }
    });
}

/// Known positive balance above unknown above zero; among positives, larger first.
fn balance_rank(row: &TokenRow) -> (u8, TokenAmount) {
    match row.balance() {
        Some(0) => (0, 0),
        None => (1, 0),
        Some(balance) => (2, balance),
    }
}

/// Whether the filter box and sort control are shown for a list of `rows`
/// tokens; below that the list is short enough to scan.
pub fn filter_shown(rows: usize) -> bool {
    rows >= MANY_TOKENS_THRESHOLD
}

/// Whether `row` stays listed under the current controls. The hide-zero
/// control hides only a fetched zero, never an unknown balance. The
/// remembered filter applies only while its box is shown, so a filter typed
/// on a long list never hides rows the user has no control to clear.
pub fn row_is_listed(row: &TokenRow, prefs: &IdentityTokensViewPrefs, filter_shown: bool) -> bool {
    if prefs.hide_zero_balances && row.balance() == Some(0) {
        return false;
    }
    !filter_shown || row.matches_filter(&prefs.filter)
}

/// Whether Home shows its Tokens card: only when the identity holds a token
/// or has something to claim.
pub fn home_card_shows(rows: &[TokenRow]) -> bool {
    rows.iter()
        .any(|row| row.holds_some() || row.claim_available())
}

/// Whether an arrival refresh is due: on arrival, when no refresh finished
/// within [`ARRIVAL_REFRESH_AFTER`].
pub fn arrival_refresh_due(arriving: bool, now_ms: u64, last_refreshed_at_ms: Option<u64>) -> bool {
    arriving
        && last_refreshed_at_ms.is_none_or(|last| {
            now_ms.saturating_sub(last) >= ARRIVAL_REFRESH_AFTER.as_millis() as u64
        })
}

/// Why the last balance refresh the hub asked for failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshFailure {
    /// The network took too long.
    TooSlow,
    /// Anything else; the technical detail is in the log.
    Failed,
}

impl RefreshFailure {
    fn from_error(error: &TaskError) -> Self {
        match error {
            TaskError::TokenBalanceRefreshTimeout { .. }
            | TaskError::TokenBalanceSyncStillRunning => RefreshFailure::TooSlow,
            _ => RefreshFailure::Failed,
        }
    }
}

/// What the Tokens tab shows and waits on for the selected identity.
#[derive(Default)]
pub struct IdentityTokensState {
    owner: Option<(Network, Identifier)>,
    rows: Arc<[TokenRow]>,
    rows_read_at: Option<Instant>,
    /// Tokens exist in the registry even when every row is dismissed.
    registry_has_tokens: bool,
    /// Pairs whose last explicit check failed with no balance to show.
    failed_checks: BTreeSet<IdentityTokenIdentifier>,
    /// The last full refresh failed, so every unknown balance could not be checked.
    full_refresh_failed: bool,
    refresh: PendingRequest,
    pair_checks: PendingRequests<IdentityTokenIdentifier>,
    stop_tracking: PendingRequests<IdentityTokenIdentifier>,
    track_again: PendingRequest<Vec<IdentityTokenIdentifier>>,
    /// A "track again" refused while a refresh ran, to send once it ends.
    track_again_owed: Option<Vec<IdentityTokenIdentifier>>,
    /// A check refused while a refresh ran; a refresh is owed once it ends.
    retry_owed: bool,
    last_refreshed_at_ms: Option<u64>,
    refresh_failure: Option<RefreshFailure>,
    arriving: bool,
    /// Set when the error just recorded belongs to a request of this tab, so
    /// the hub shows no global banner for it.
    swallow_next_error: bool,
    prefs: Option<IdentityTokensViewPrefs>,
    /// The list controls changed since they were last written to the device.
    prefs_dirty: bool,
    /// The Stop tracking confirmation, with the pair it is about.
    pub stop_tracking_dialog: Option<(ConfirmationDialog, IdentityTokenIdentifier)>,
    /// The token whose detail page is open instead of the list.
    pub detail: Option<IdentityTokenIdentifier>,
    /// The refresh error banner was dismissed; rows keep their stale time.
    pub failure_dismissed: bool,
}

/// What the arrival rule asks for on a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arrival {
    /// Balances are fresh enough; the arrival is spent.
    NotDue,
    /// A refresh is wanted but cannot go out yet; the arrival stays owed.
    Owed,
    /// Refresh now.
    Due,
}

impl IdentityTokensState {
    /// Drop what belongs to the previous identity: its rows, failed checks
    /// and open dialogs. Everything app-wide survives — device preferences,
    /// the in-flight requests (so a late answer is still this tab's), and the
    /// time of the last refresh, which covered every identity.
    pub fn reset_for_identity_change(&mut self) {
        *self = Self {
            prefs: self.prefs.take(),
            prefs_dirty: self.prefs_dirty,
            refresh: std::mem::take(&mut self.refresh),
            pair_checks: std::mem::take(&mut self.pair_checks),
            stop_tracking: std::mem::take(&mut self.stop_tracking),
            track_again: std::mem::take(&mut self.track_again),
            // Owed pairs belong to the previous identity; its next visit
            // offers "track again" anew.
            track_again_owed: None,
            retry_owed: self.retry_owed,
            last_refreshed_at_ms: self.last_refreshed_at_ms,
            refresh_failure: self.refresh_failure,
            failure_dismissed: self.failure_dismissed,
            full_refresh_failed: self.full_refresh_failed,
            arriving: true,
            ..Self::default()
        };
    }

    /// Bind the rows to the identity on screen, dropping another identity's.
    /// A filter still being edited is written to the device first, so a
    /// switch never loses it.
    fn reconcile_owner(&mut self, app_context: &AppContext, identity: &QualifiedIdentity) {
        let owner = Some((identity.network, identity.identity.id()));
        if self.owner != owner {
            self.save_prefs(app_context);
            self.reset_for_identity_change();
            self.owner = owner;
        }
    }

    /// The hub was just shown: re-read the rows and apply the arrival rule.
    pub fn arrive(&mut self) {
        self.arriving = true;
        self.invalidate_rows();
    }

    /// Re-read the rows on the next frame.
    pub fn invalidate_rows(&mut self) {
        self.rows_read_at = None;
    }

    /// The identity's rows, re-read when stale or invalidated. Cheap to clone.
    pub fn rows(
        &mut self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
    ) -> Arc<[TokenRow]> {
        self.reconcile_owner(app_context, identity);
        let stale = self
            .rows_read_at
            .is_none_or(|read_at| read_at.elapsed() >= ROWS_TTL);
        if stale {
            self.reread_rows(app_context, identity);
        }
        Arc::clone(&self.rows)
    }

    /// How many of the identity's tokens have something to claim, for the tab
    /// badge on every tab. Reads the registry only when the rows were never
    /// read or were invalidated by an event; the time-based re-read is left
    /// to the surfaces that show the rows.
    pub fn badge_pending_claims(
        &mut self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
    ) -> usize {
        self.reconcile_owner(app_context, identity);
        if self.rows_read_at.is_none() {
            self.reread_rows(app_context, identity);
        }
        self.pending_claims()
    }

    fn reread_rows(&mut self, app_context: &AppContext, identity: &QualifiedIdentity) {
        match self.read_rows(app_context, identity) {
            Ok((rows, registry_has_tokens)) => {
                self.rows = rows.into();
                self.registry_has_tokens = registry_has_tokens;
            }
            Err(error) => {
                tracing::warn!(
                    error = ?error,
                    "Could not read the identity's token rows; keeping the ones shown"
                );
            }
        }
        self.rows_read_at = Some(Instant::now());
    }

    fn read_rows(
        &self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
    ) -> Result<(Vec<TokenRow>, bool), TaskError> {
        let identity_id = identity.identity.id();
        let registry = app_context.get_all_known_tokens_with_data_contract()?;
        let registry_has_tokens = !registry.is_empty();
        let dismissed = app_context.untracked_token_balances()?;
        let balances = app_context.identity_token_balances()?;
        let mut rows = Vec::with_capacity(registry.len());
        for (token_id, token) in registry {
            let pair = IdentityTokenIdentifier {
                identity_id,
                token_id,
            };
            if dismissed.contains(&pair) {
                continue;
            }
            let balance = balances.get(&pair).map(|record| record.balance);
            let config = &token.token_configuration;
            // Computed without the developer display override, so the admin
            // flags below answer the real permission question (Q3).
            let actions = get_available_token_actions_for_identity(
                balance,
                identity,
                config,
                &token.data_contract,
                false,
                None,
            );
            let once_claim = match once_per_identity_amount(config) {
                Some(amount) => Some(OnceClaim {
                    amount,
                    claimed_at_ms: app_context
                        .once_per_identity_claimed_at(&token_id, &identity_id)?,
                }),
                None => None,
            };
            let issuer_id = token.data_contract.owner_id();
            rows.push(TokenRow {
                issuer_id,
                issued_by_me: issuer_id == identity_id,
                has_admin_permission: actions.has_admin_permission(),
                check_failed: balance.is_none()
                    && (self.full_refresh_failed || self.failed_checks.contains(&pair)),
                once_claim,
                token: IdentityTokenMaybeBalanceWithActions {
                    token_id,
                    token_alias: token_display_name(&token.token_name, &token_id),
                    token_config: token.token_configuration,
                    identity_id,
                    identity_name: None,
                    balance,
                    estimated_unclaimed_rewards: None,
                    data_contract_id: token.data_contract.id(),
                    token_position: token.token_position,
                    available_actions: actions,
                },
            });
        }
        Ok((rows, registry_has_tokens))
    }

    /// Tokens are known on this device, whether or not this identity tracks them.
    pub fn registry_has_tokens(&self) -> bool {
        self.registry_has_tokens
    }

    /// How many of the cached rows have something to claim.
    pub fn pending_claims(&self) -> usize {
        self.rows.iter().filter(|row| row.claim_available()).count()
    }

    /// The list controls, read from the device on first use.
    pub fn prefs(&mut self, app_context: &AppContext) -> &IdentityTokensViewPrefs {
        self.prefs
            .get_or_insert_with(|| app_context.identity_tokens_view_prefs())
    }

    /// Change the list controls for this session; [`save_prefs`](Self::save_prefs)
    /// writes them to the device. The tab calls it when the filter box loses
    /// focus or another control changes, the hub when the tab is left, and
    /// the state itself when the identity changes.
    pub fn edit_prefs(&mut self, mut prefs: IdentityTokensViewPrefs) {
        prefs.clamp_filter();
        if self.prefs.as_ref() != Some(&prefs) {
            self.prefs = Some(prefs);
            self.prefs_dirty = true;
        }
    }

    /// Remember the list controls on this device, if they changed.
    pub fn save_prefs(&mut self, app_context: &AppContext) {
        if !self.prefs_dirty {
            return;
        }
        let Some(prefs) = &self.prefs else {
            return;
        };
        if let Err(error) = app_context.save_identity_tokens_view_prefs(prefs) {
            tracing::warn!(
                error = ?error,
                "Could not save the Identity Hub token list preferences; keeping them for this session"
            );
        }
        self.prefs_dirty = false;
    }

    /// Change the list controls and remember them on this device at once.
    pub fn set_prefs(&mut self, app_context: &AppContext, prefs: IdentityTokensViewPrefs) {
        self.edit_prefs(prefs);
        self.save_prefs(app_context);
    }

    /// The list controls changed since they were last written to the device.
    pub fn prefs_dirty(&self) -> bool {
        self.prefs_dirty
    }

    /// When balances were last refreshed while this tab was watching (Unix ms).
    pub fn last_refreshed_at_ms(&self) -> Option<u64> {
        self.last_refreshed_at_ms
    }

    /// Why the last refresh failed, until the next success or dismissal.
    pub fn refresh_failure(&self) -> Option<RefreshFailure> {
        if self.failure_dismissed {
            None
        } else {
            self.refresh_failure
        }
    }

    /// This tab is waiting on a refresh it asked for.
    pub fn refresh_pending(&self) -> bool {
        self.refresh.is_pending() || self.track_again.is_pending()
    }

    /// A balance refresh is running, here or anywhere else in the app.
    pub fn refreshing(&self, app_context: &AppContext) -> bool {
        self.refresh_pending() || app_context.token_balance_refresh_in_flight()
    }

    /// Whether the first-load line shows: every balance is still unknown, a
    /// refresh is actually running, and nothing has failed. A failed or
    /// impossible refresh shows its own message instead.
    pub fn first_load_shown(&self, rows: &[TokenRow], refreshing: bool) -> bool {
        !rows.is_empty()
            && rows.iter().all(|row| row.balance().is_none())
            && refreshing
            && self.refresh_failure().is_none()
    }

    /// Cached data changed under a task of any screen: re-read the rows on
    /// the next frame when they may be affected. Token balances and the
    /// identities that own them both feed the rows.
    pub fn cached_data_changed(&mut self, domain: crate::backend_task::CacheDomain) {
        use crate::backend_task::CacheDomain;
        if matches!(
            domain,
            CacheDomain::Tokens | CacheDomain::Identities | CacheDomain::All
        ) {
            self.invalidate_rows();
        }
    }

    /// Proof-verifying reads sent before the masternode list is synced fail
    /// and get the queried nodes banned, so no network read goes out before.
    fn can_read(app_context: &AppContext) -> bool {
        app_context.connection_status().masternodes_ready()
    }

    /// The arrival rule for this frame. A refresh owed after an in-flight
    /// refusal counts like an arrival.
    fn arrival(&self, now_ms: u64, synced: bool, refresh_running: bool) -> Arrival {
        let wanted = self.retry_owed
            || self.track_again_owed.is_some()
            || arrival_refresh_due(self.arriving, now_ms, self.last_refreshed_at_ms);
        if !wanted {
            return Arrival::NotDue;
        }
        // A refresh already running, here or on another screen, reports when
        // it ends; an unsynced network cannot be asked yet.
        if !synced || refresh_running {
            return Arrival::Owed;
        }
        Arrival::Due
    }

    /// The arrival refresh, when due. Claims the hub's dispatch slot, so the
    /// caller asks only on a frame nothing else produced an action.
    pub fn load_action(&mut self, app_context: &AppContext, now_ms: u64) -> AppAction {
        let refresh_running = self.refreshing(app_context);
        match self.arrival(now_ms, Self::can_read(app_context), refresh_running) {
            Arrival::NotDue => {
                self.arriving = false;
                AppAction::None
            }
            Arrival::Owed => AppAction::None,
            Arrival::Due => {
                self.arriving = false;
                self.retry_owed = false;
                match self.track_again_owed.take() {
                    Some(pairs) => self.dispatch_track_again(pairs),
                    None => self.refresh_action(app_context),
                }
            }
        }
    }

    /// Ask for a balance refresh now, unless the network is not synced or one
    /// is already running anywhere in the app — the guard is shared with My
    /// Tokens.
    pub fn refresh_action(&mut self, app_context: &AppContext) -> AppAction {
        if !Self::can_read(app_context) || self.refreshing(app_context) {
            return AppAction::None;
        }
        self.failure_dismissed = false;
        self.dispatch_refresh()
    }

    fn dispatch_refresh(&mut self) -> AppAction {
        self.refresh.dispatch(BackendTask::TokenTask(Box::new(
            TokenTask::QueryMyTokenBalances,
        )))
    }

    /// Check one pair's balance now; also tracks the pair again. Waits for the
    /// masternode list and for the app-wide refresh guard like every other
    /// network read.
    pub fn check_pair_action(
        &mut self,
        app_context: &AppContext,
        pair: IdentityTokenIdentifier,
    ) -> AppAction {
        if !Self::can_read(app_context) || self.refreshing(app_context) {
            return AppAction::None;
        }
        self.dispatch_pair_check(pair)
    }

    fn dispatch_pair_check(&mut self, pair: IdentityTokenIdentifier) -> AppAction {
        self.pair_checks.dispatch_with(
            BackendTask::TokenTask(Box::new(TokenTask::QueryIdentityTokenBalance(pair))),
            pair,
        )
    }

    /// Stop tracking `pair` for this identity; its row leaves at once.
    pub fn stop_tracking_action(&mut self, pair: IdentityTokenIdentifier) -> AppAction {
        self.rows = self
            .rows
            .iter()
            .filter(|row| row.pair() != pair)
            .cloned()
            .collect();
        if self.detail == Some(pair) {
            self.detail = None;
        }
        self.stop_tracking.dispatch_with(
            BackendTask::TokenTask(Box::new(TokenTask::StopTrackingTokenBalance(pair))),
            pair,
        )
    }

    /// Track every dismissed token of `identity` again, in one request this
    /// tab waits on. Waits for the masternode list and the refresh guard like
    /// every network read.
    pub fn track_again_action(
        &mut self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
    ) -> AppAction {
        if !Self::can_read(app_context) || self.refreshing(app_context) {
            return AppAction::None;
        }
        let identity_id = identity.identity.id();
        let dismissed = match app_context.untracked_token_balances() {
            Ok(dismissed) => dismissed,
            Err(error) => {
                tracing::warn!(error = ?error, "Could not read the dismissed token pairs");
                return AppAction::None;
            }
        };
        let pairs: Vec<IdentityTokenIdentifier> = dismissed
            .into_iter()
            .filter(|pair| pair.identity_id == identity_id)
            .collect();
        if pairs.is_empty() {
            return AppAction::None;
        }
        self.failure_dismissed = false;
        self.dispatch_track_again(pairs)
    }

    fn dispatch_track_again(&mut self, pairs: Vec<IdentityTokenIdentifier>) -> AppAction {
        self.invalidate_rows();
        self.track_again.dispatch_with(
            BackendTask::TokenTask(Box::new(TokenTask::TrackTokenBalancesAgain(pairs.clone()))),
            pairs,
        )
    }

    /// Settle a result of this tab's requests. Returns whether it was one.
    pub fn record_result(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
        now_ms: u64,
    ) -> bool {
        // Only the full refresh fetched every identity's balances; "track
        // again" refreshed the selected identity's watch set alone.
        if self.refresh.settle(context).is_some() {
            if matches!(result, BackendTaskSuccessResult::FetchedTokenBalances) {
                self.note_full_refresh(now_ms);
            }
            self.invalidate_rows();
            return true;
        }
        if let Some(pairs) = self.track_again.settle(context) {
            for pair in &pairs {
                self.failed_checks.remove(pair);
            }
            self.invalidate_rows();
            return true;
        }
        if let Some(pair) = self.pair_checks.settle(context) {
            self.failed_checks.remove(&pair);
            self.invalidate_rows();
            return true;
        }
        if self.stop_tracking.settle(context).is_some() {
            self.invalidate_rows();
            return true;
        }
        false
    }

    /// Every balance was fetched just now, by this tab or another screen.
    fn note_full_refresh(&mut self, now_ms: u64) {
        self.last_refreshed_at_ms = Some(now_ms);
        self.refresh_failure = None;
        self.full_refresh_failed = false;
        self.retry_owed = false;
    }

    /// Note a failure of this tab's requests. A refresh already running
    /// elsewhere, or a read held back until the network has synced, is not a
    /// failure: the request is owed again once it can go out. Anything else
    /// is shown inline, never as a global banner.
    pub fn record_error(&mut self, context: &BackendTaskContext, error: &TaskError) {
        let in_progress = matches!(
            error,
            TaskError::TokenBalanceRefreshInProgress | TaskError::MasternodeListSyncPending
        );
        if let Some(pairs) = self.track_again.settle(context) {
            if in_progress {
                self.track_again_owed = Some(pairs);
            } else {
                // Only the selected identity's pairs were refreshed: mark
                // those, never every identity's unknown balances.
                tracing::warn!(
                    error = ?error,
                    "Tracking token balances again for the Identity Hub failed"
                );
                self.failed_checks.extend(pairs);
            }
            self.invalidate_rows();
            self.swallow_next_error = true;
            return;
        }
        if self.refresh.settle(context).is_some() {
            if in_progress {
                self.retry_owed = true;
            } else {
                self.note_full_refresh_failed(error);
            }
            self.invalidate_rows();
            self.swallow_next_error = true;
            return;
        }
        if let Some(pair) = self.pair_checks.settle(context) {
            if in_progress {
                self.retry_owed = true;
            } else {
                tracing::warn!(
                    error = ?error,
                    token = %pair.token_id,
                    "Checking one token balance for the Identity Hub failed"
                );
                self.failed_checks.insert(pair);
                self.invalidate_rows();
            }
            self.swallow_next_error = true;
            return;
        }
        if self.stop_tracking.settle(context).is_some() {
            // The row left optimistically; put it back with the default banner.
            self.invalidate_rows();
        }
    }

    fn note_full_refresh_failed(&mut self, error: &TaskError) {
        tracing::warn!(error = ?error, "The Identity Hub token balance refresh failed");
        self.refresh_failure = Some(RefreshFailure::from_error(error));
        self.failure_dismissed = false;
        self.full_refresh_failed = true;
    }

    /// Whether the error just recorded is shown by this tab rather than a banner.
    pub fn take_swallowed_error(&mut self) -> bool {
        std::mem::take(&mut self.swallow_next_error)
    }

    /// Another screen's refresh fetched every balance just now: as fresh as
    /// one of this tab's own, so the arrival rule is satisfied too.
    pub fn balances_refreshed_elsewhere(&mut self, now_ms: u64) {
        self.note_full_refresh(now_ms);
        self.invalidate_rows();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::token::IdentityTokenAvailableActions;
    use dash_sdk::dpp::data_contract::TokenConfiguration;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v1::TokenConfigurationV1Setters;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;

    fn row(name: &str, balance: Option<TokenAmount>) -> TokenRow {
        TokenRow {
            token: IdentityTokenMaybeBalanceWithActions {
                token_id: Identifier::from([name.len() as u8; 32]),
                token_alias: name.into(),
                token_config: TokenConfiguration::V0(
                    TokenConfigurationV0::default_most_restrictive(),
                ),
                identity_id: Identifier::from([1; 32]),
                identity_name: None,
                balance,
                estimated_unclaimed_rewards: None,
                data_contract_id: Identifier::from([9; 32]),
                token_position: 0,
                available_actions: IdentityTokenAvailableActions::default(),
            },
            issuer_id: Identifier::from([8; 32]),
            issued_by_me: false,
            once_claim: None,
            has_admin_permission: false,
            check_failed: false,
        }
    }

    fn claimable(mut row: TokenRow) -> TokenRow {
        row.token.available_actions.can_claim = true;
        row
    }

    fn names(rows: &[TokenRow]) -> Vec<&str> {
        rows.iter().map(TokenRow::name).collect()
    }

    fn dispatched(action: AppAction) -> BackendTaskContext {
        match action {
            AppAction::BackendTaskWithContext { context, .. } => context,
            other => panic!("expected a dispatch with its own context, got {other:?}"),
        }
    }

    #[test]
    fn claimable_first_then_balance_with_unknown_before_zero() {
        let mut rows = vec![
            row("Zero", Some(0)),
            row("Small", Some(1)),
            row("Checking", None),
            row("Big", Some(1_000)),
            claimable(row("Claimable", Some(0))),
        ];
        sort_rows(&mut rows, TokenListSort::ClaimableFirst);
        assert_eq!(
            names(&rows),
            ["Claimable", "Big", "Small", "Checking", "Zero"]
        );
    }

    #[test]
    fn a_taken_once_claim_is_no_longer_claimable_and_sorts_by_balance() {
        let mut taken = claimable(row("Taken", Some(100)));
        taken.once_claim = Some(OnceClaim {
            amount: 100,
            claimed_at_ms: Some(1),
        });
        let mut rows = vec![taken, row("Bigger", Some(200))];
        sort_rows(&mut rows, TokenListSort::ClaimableFirst);
        assert_eq!(names(&rows), ["Bigger", "Taken"]);
    }

    #[test]
    fn sorting_borrowed_rows_works_the_same() {
        let rows = [row("b", Some(1)), row("A", None), row("c", Some(5))];
        let mut refs: Vec<&TokenRow> = rows.iter().collect();
        sort_rows(&mut refs, TokenListSort::BalanceHighToLow);
        assert_eq!(
            refs.iter().map(|row| row.name()).collect::<Vec<_>>(),
            ["c", "b", "A"]
        );
        sort_rows(&mut refs, TokenListSort::NameAToZ);
        assert_eq!(
            refs.iter().map(|row| row.name()).collect::<Vec<_>>(),
            ["A", "b", "c"]
        );
    }

    #[test]
    fn hide_zero_hides_a_fetched_zero_but_never_an_unknown_balance() {
        let prefs = IdentityTokensViewPrefs {
            hide_zero_balances: true,
            ..Default::default()
        };
        assert!(!row_is_listed(&row("Zero", Some(0)), &prefs, true));
        assert!(row_is_listed(&row("Checking", None), &prefs, true));
        assert!(row_is_listed(&row("Held", Some(3)), &prefs, true));
        assert!(row_is_listed(
            &row("Zero", Some(0)),
            &IdentityTokensViewPrefs::default(),
            true
        ));
    }

    #[test]
    fn filter_matches_name_or_id_case_insensitively() {
        let prefs = |filter: &str| IdentityTokensViewPrefs {
            filter: filter.into(),
            ..Default::default()
        };
        let coffee = row("DashCoffee", Some(1));
        assert!(row_is_listed(&coffee, &prefs("coffee"), true));
        assert!(row_is_listed(&coffee, &prefs("  "), true));
        assert!(row_is_listed(
            &coffee,
            &prefs(&coffee.token_id_base58()[..6].to_lowercase()),
            true
        ));
        assert!(!row_is_listed(&coffee, &prefs("points"), true));
    }

    #[test]
    fn the_filter_applies_only_while_its_box_is_shown() {
        let prefs = IdentityTokensViewPrefs {
            filter: "coffee".into(),
            ..Default::default()
        };
        let grit = row("Grit", Some(3));
        assert!(!filter_shown(MANY_TOKENS_THRESHOLD - 1));
        assert!(filter_shown(MANY_TOKENS_THRESHOLD));
        assert!(row_is_listed(&grit, &prefs, false));
        assert!(!row_is_listed(&grit, &prefs, true));
        assert!(row_is_listed(&row("DashCoffee", Some(1)), &prefs, true));
    }

    #[test]
    fn status_prefers_checking_then_key_then_claims() {
        assert_eq!(row_status(&row("T", None), false), RowStatus::Checking);
        let mut failed = row("T", None);
        failed.check_failed = true;
        assert_eq!(row_status(&failed, false), RowStatus::CouldNotCheck);

        assert_eq!(row_status(&row("T", Some(5)), false), RowStatus::None);
        assert_eq!(row_status(&row("T", Some(5)), true), RowStatus::AddKey);
        assert_eq!(
            row_status(&row("T", Some(0)), true),
            RowStatus::None,
            "nothing to send or claim, so no key is needed"
        );

        let mut once = claimable(row("AIR", Some(0)));
        once.once_claim = Some(OnceClaim {
            amount: 100,
            claimed_at_ms: None,
        });
        assert_eq!(
            row_status(&once, false),
            RowStatus::CanClaimOnce(once.amount(100))
        );
        once.once_claim = Some(OnceClaim {
            amount: 100,
            claimed_at_ms: Some(1_700_000_000_000),
        });
        assert!(matches!(row_status(&once, false), RowStatus::ClaimedOn(_)));

        let rewards = claimable(row("DCF", Some(5)));
        assert_eq!(row_status(&rewards, false), RowStatus::RewardsMayBeWaiting);
    }

    #[test]
    fn a_taken_once_claim_shows_its_local_date() {
        let mut taken = row("Taken", Some(1));
        taken.once_claim = Some(OnceClaim {
            amount: 100,
            claimed_at_ms: Some(1_700_000_000_000),
        });
        let expected = crate::model::datetime::instant_from_unix_millis(1_700_000_000_000)
            .map(crate::model::datetime::local_date)
            .expect("a representable instant");
        assert_eq!(taken.claimed_on_date(), Some(expected.clone()));
        assert_eq!(row_status(&taken, false), RowStatus::ClaimedOn(expected));
    }

    #[test]
    fn a_non_transferable_zero_row_needs_no_key() {
        let mut badge = row("FAN", Some(1));
        badge.token.token_config.set_transferable(false);
        assert!(!badge.transferable());
        assert_eq!(row_status(&badge, true), RowStatus::None);
    }

    #[test]
    fn home_card_needs_a_balance_or_a_claim() {
        assert!(!home_card_shows(&[]));
        assert!(!home_card_shows(&[
            row("Zero", Some(0)),
            row("Checking", None)
        ]));
        assert!(home_card_shows(&[
            row("Zero", Some(0)),
            row("Held", Some(1))
        ]));
        assert!(home_card_shows(&[claimable(row("Air", Some(0)))]));
    }

    #[test]
    fn arrival_refresh_is_due_when_never_or_old() {
        let two_minutes = ARRIVAL_REFRESH_AFTER.as_millis() as u64;
        assert!(arrival_refresh_due(true, 0, None));
        assert!(arrival_refresh_due(true, two_minutes, Some(0)));
        assert!(!arrival_refresh_due(true, two_minutes - 1, Some(0)));
        assert!(!arrival_refresh_due(false, two_minutes, Some(0)));
    }

    #[test]
    fn pending_claims_counts_claimable_rows_only() {
        let mut state = IdentityTokensState::default();
        let mut taken = claimable(row("B", Some(0)));
        taken.once_claim = Some(OnceClaim {
            amount: 1,
            claimed_at_ms: Some(1),
        });
        state.rows = vec![claimable(row("A", Some(0))), taken, row("C", Some(4))].into();
        assert_eq!(state.pending_claims(), 1);
    }

    #[test]
    fn a_refresh_result_records_the_time() {
        let mut state = IdentityTokensState::default();
        let context = dispatched(state.dispatch_refresh());
        assert!(state.refresh_pending());
        assert!(!state.record_result(
            &BackendTaskContext::TokenBalanceRefresh,
            &BackendTaskSuccessResult::FetchedTokenBalances,
            7,
        ));
        assert!(state.refresh_pending(), "another dispatch settles nothing");
        assert!(state.record_result(&context, &BackendTaskSuccessResult::FetchedTokenBalances, 7));
        assert_eq!(state.last_refreshed_at_ms(), Some(7));
        assert!(!state.refresh_pending());
    }

    #[test]
    fn a_settled_task_is_no_longer_pending_whatever_it_returned() {
        let mut state = IdentityTokensState::default();
        let context = dispatched(state.dispatch_refresh());
        state.record_result(&context, &BackendTaskSuccessResult::SavedToken, 7);
        assert!(!state.refresh_pending());
        assert_eq!(state.last_refreshed_at_ms(), None);

        let context = dispatched(state.dispatch_refresh());
        state.record_error(&context, &TaskError::TokenBalanceRefreshInProgress);
        assert!(!state.refresh_pending());
    }

    #[test]
    fn an_arrival_stays_owed_until_the_network_is_synced_and_no_refresh_runs() {
        let mut state = IdentityTokensState::default();
        state.arrive();
        assert_eq!(state.arrival(0, false, false), Arrival::Owed);
        assert_eq!(state.arrival(0, true, true), Arrival::Owed);
        assert_eq!(state.arrival(0, true, false), Arrival::Due);
        state.balances_refreshed_elsewhere(0);
        assert_eq!(
            state.arrival(1, true, false),
            Arrival::NotDue,
            "another screen's fetch satisfies the arrival"
        );
    }

    #[test]
    fn an_in_progress_refresh_is_not_a_failure_but_a_timeout_is() {
        let mut state = IdentityTokensState::default();
        let context = dispatched(state.dispatch_refresh());
        state.record_error(&context, &TaskError::TokenBalanceRefreshInProgress);
        assert_eq!(state.refresh_failure(), None);
        assert!(state.take_swallowed_error());

        let context = dispatched(state.dispatch_refresh());
        let timeout = tokio::runtime::Runtime::new()
            .expect("runtime")
            .block_on(async {
                tokio::time::timeout(Duration::ZERO, std::future::pending::<()>()).await
            })
            .expect_err("elapsed");
        state.record_error(
            &context,
            &TaskError::TokenBalanceRefreshTimeout { source: timeout },
        );
        assert_eq!(state.refresh_failure(), Some(RefreshFailure::TooSlow));
        state.failure_dismissed = true;
        assert_eq!(state.refresh_failure(), None);
    }

    /// A failed full refresh leaves every unknown balance "could not check",
    /// and the first-load line never shows next to a failure.
    #[test]
    fn a_failed_full_refresh_marks_unknown_balances_and_ends_the_first_load() {
        let mut state = IdentityTokensState::default();
        let rows = [row("A", None), row("B", None)];
        assert!(state.first_load_shown(&rows, true));
        assert!(!state.first_load_shown(&rows, false), "nothing is running");
        assert!(
            !state.first_load_shown(&[row("A", Some(1)), row("B", None)], true),
            "a known balance ends the first load"
        );

        let context = dispatched(state.dispatch_refresh());
        state.record_error(&context, &TaskError::NoIdentitiesFound);
        assert_eq!(state.refresh_failure(), Some(RefreshFailure::Failed));
        assert!(state.full_refresh_failed);
        assert!(!state.first_load_shown(&rows, true));

        state.balances_refreshed_elsewhere(5);
        assert!(!state.full_refresh_failed);
        assert_eq!(state.refresh_failure(), None);
    }

    #[test]
    fn a_failed_pair_check_marks_the_row_until_it_succeeds() {
        let mut state = IdentityTokensState::default();
        let pair = row("T", None).pair();
        let context = dispatched(state.dispatch_pair_check(pair));
        state.record_error(&context, &TaskError::NoIdentitiesFound);
        assert!(state.failed_checks.contains(&pair));
        assert!(state.take_swallowed_error());

        let context = dispatched(state.dispatch_pair_check(pair));
        assert!(state.record_result(&context, &BackendTaskSuccessResult::FetchedTokenBalances, 1));
        assert!(!state.failed_checks.contains(&pair));
    }

    #[test]
    fn stop_tracking_removes_the_row_at_once() {
        let mut state = IdentityTokensState {
            rows: vec![row("A", Some(1)), row("Bee", Some(2))].into(),
            ..Default::default()
        };
        let pair = state.rows[0].pair();
        state.detail = Some(pair);
        let _ = state.stop_tracking_action(pair);
        assert_eq!(names(&state.rows), ["Bee"]);
        assert_eq!(state.detail, None);
    }

    /// In-flight bookkeeping survives an identity switch, so an answer that
    /// arrives afterwards is still this tab's, never a global banner.
    #[test]
    fn a_refresh_error_after_an_identity_change_is_still_this_tabs() {
        let mut state = IdentityTokensState::default();
        let context = dispatched(state.dispatch_refresh());
        state.reset_for_identity_change();
        state.record_error(&context, &TaskError::TokenBalanceRefreshInProgress);
        assert!(state.take_swallowed_error());
    }

    /// A balance refresh covers every identity, so switching identity keeps
    /// its time and does not ask the network again within the arrival window.
    #[test]
    fn an_identity_switch_keeps_the_refresh_time() {
        let mut state = IdentityTokensState::default();
        state.balances_refreshed_elsewhere(1_000);
        state.reset_for_identity_change();
        assert_eq!(state.last_refreshed_at_ms(), Some(1_000));
        assert_eq!(state.arrival(2_000, true, false), Arrival::NotDue);
    }

    /// Two quick checks on different rows are both this tab's: neither error
    /// becomes a global banner and no check stays outstanding.
    #[test]
    fn a_second_pair_check_does_not_orphan_the_first() {
        let mut state = IdentityTokensState::default();
        let first_ctx = dispatched(state.dispatch_pair_check(row("First", None).pair()));
        let second_ctx = dispatched(state.dispatch_pair_check(row("Second", None).pair()));
        state.record_error(&first_ctx, &TaskError::NoIdentitiesFound);
        assert!(
            state.take_swallowed_error(),
            "the first check is still ours"
        );
        state.record_error(&second_ctx, &TaskError::NoIdentitiesFound);
        assert!(state.take_swallowed_error());
        assert!(
            !state.pair_checks.is_pending(),
            "nothing of this tab is outstanding"
        );
    }

    /// A check refused because a refresh already runs is retried once that
    /// refresh ends, instead of leaving the row on "Checking balance…".
    #[test]
    fn an_in_flight_refusal_re_arms_a_refresh() {
        let mut state = IdentityTokensState::default();
        state.balances_refreshed_elsewhere(1_000);
        let context = dispatched(state.dispatch_pair_check(row("T", None).pair()));
        state.record_error(&context, &TaskError::TokenBalanceRefreshInProgress);
        assert!(state.take_swallowed_error());
        assert_eq!(state.arrival(1_001, true, true), Arrival::Owed);
        assert_eq!(state.arrival(1_001, true, false), Arrival::Due);
    }

    /// "Track tokens again" refused while a refresh ran is sent again, with
    /// the same pairs, once the refresh ends.
    #[test]
    fn a_refused_track_again_is_sent_again_with_its_pairs() {
        let mut state = IdentityTokensState::default();
        state.balances_refreshed_elsewhere(1_000);
        let pairs = vec![row("A", None).pair(), row("Bee", None).pair()];
        let context = dispatched(state.dispatch_track_again(pairs.clone()));
        state.record_error(&context, &TaskError::TokenBalanceRefreshInProgress);
        assert!(state.take_swallowed_error());
        assert_eq!(state.track_again_owed, Some(pairs.clone()));
        assert_eq!(state.arrival(1_001, true, false), Arrival::Due);
        assert_eq!(state.arrival(1_001, true, true), Arrival::Owed);
    }

    /// A refresh refused because the network is still syncing is sent again
    /// once it can go out, even when the last refresh is still recent.
    #[test]
    fn a_refresh_refused_before_sync_is_sent_again_once_synced() {
        let mut state = IdentityTokensState::default();
        state.balances_refreshed_elsewhere(1_000);
        let context = dispatched(state.dispatch_refresh());
        state.record_error(&context, &TaskError::MasternodeListSyncPending);
        assert!(state.take_swallowed_error());
        assert_eq!(state.refresh_failure(), None, "waiting is not a failure");
        assert_eq!(state.arrival(1_001, false, false), Arrival::Owed);
        assert_eq!(state.arrival(1_001, true, false), Arrival::Due);
    }

    /// A background balance check that outlasted the wait reads as "taking
    /// longer than usual", not as a connection failure.
    #[test]
    fn a_background_check_still_running_reads_as_slow() {
        let mut state = IdentityTokensState::default();
        let context = dispatched(state.dispatch_refresh());
        state.record_error(&context, &TaskError::TokenBalanceSyncStillRunning);
        assert_eq!(state.refresh_failure(), Some(RefreshFailure::TooSlow));
    }

    /// "Track tokens again" refreshes only the selected identity's watch set,
    /// so it never counts as the full refresh another identity relies on.
    #[test]
    fn a_track_again_result_is_not_a_full_refresh() {
        let mut state = IdentityTokensState::default();
        let context = dispatched(state.dispatch_track_again(vec![row("A", None).pair()]));
        assert!(state.record_result(&context, &BackendTaskSuccessResult::FetchedTokenBalances, 7));
        assert!(!state.refresh_pending());
        assert_eq!(state.last_refreshed_at_ms(), None);

        state.reset_for_identity_change();
        assert_eq!(
            state.arrival(8, true, false),
            Arrival::Due,
            "the next identity still needs its own refresh"
        );
    }

    /// A failed "track again" marks only its own pairs "could not check": it
    /// is no full refresh, so another identity's unknown balances stay
    /// unmarked after a switch. A later success clears the pairs again.
    #[test]
    fn a_failed_track_again_marks_only_its_pairs() {
        let mut state = IdentityTokensState::default();
        let pairs = vec![row("A", None).pair(), row("Bee", None).pair()];
        let context = dispatched(state.dispatch_track_again(pairs.clone()));
        state.record_error(&context, &TaskError::NoIdentitiesFound);
        assert!(state.take_swallowed_error());
        assert!(!state.full_refresh_failed);
        assert_eq!(state.refresh_failure(), None);
        assert!(pairs.iter().all(|pair| state.failed_checks.contains(pair)));

        let context = dispatched(state.dispatch_track_again(pairs.clone()));
        assert!(state.record_result(&context, &BackendTaskSuccessResult::FetchedTokenBalances, 3));
        assert!(state.failed_checks.is_empty());

        let context = dispatched(state.dispatch_track_again(pairs));
        state.record_error(&context, &TaskError::NoIdentitiesFound);
        state.reset_for_identity_change();
        assert!(!state.full_refresh_failed);
        assert_eq!(state.refresh_failure(), None);
        assert!(state.failed_checks.is_empty());
    }

    /// Owed "track again" pairs belong to the identity that asked; switching
    /// identity drops them instead of retrying them for the wrong identity.
    #[test]
    fn an_identity_switch_drops_an_owed_track_again() {
        let mut state = IdentityTokensState::default();
        state.balances_refreshed_elsewhere(1_000);
        let context = dispatched(state.dispatch_track_again(vec![row("A", None).pair()]));
        state.record_error(&context, &TaskError::TokenBalanceRefreshInProgress);
        assert!(state.track_again_owed.is_some());

        state.reset_for_identity_change();
        assert_eq!(state.track_again_owed, None);
        assert_eq!(
            state.arrival(1_001, true, false),
            Arrival::NotDue,
            "nothing is owed for the new identity"
        );
    }

    /// A filter still being typed when the identity changes is written to
    /// the device, never lost.
    #[test]
    fn switching_identity_saves_a_filter_still_being_edited() {
        let dir = tempfile::tempdir().unwrap();
        let context = crate::context::test_support::test_app_context(dir.path());
        context.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(Arc::new(
            crate::wallet_backend::kv_test_support::InMemoryKv::default(),
        )));
        let identity = |byte: u8| {
            crate::context::test_support::bare_user_identity(
                Identifier::from([byte; 32]),
                Network::Testnet,
            )
        };
        let mut state = IdentityTokensState::default();
        state.rows(&context, &identity(1));
        state.prefs(&context);
        state.edit_prefs(IdentityTokensViewPrefs {
            filter: "cof".into(),
            ..Default::default()
        });
        assert_eq!(context.identity_tokens_view_prefs().filter, "");
        state.rows(&context, &identity(2));
        assert_eq!(context.identity_tokens_view_prefs().filter, "cof");
        assert!(!state.prefs_dirty());
    }

    #[test]
    fn edited_prefs_are_dirty_until_saved_and_the_filter_is_capped() {
        let mut state = IdentityTokensState::default();
        assert!(!state.prefs_dirty());
        state.edit_prefs(IdentityTokensViewPrefs {
            filter: "x".repeat(IdentityTokensViewPrefs::MAX_FILTER_CHARS + 10),
            ..Default::default()
        });
        assert!(state.prefs_dirty());
        assert_eq!(
            state
                .prefs
                .as_ref()
                .map(|prefs| prefs.filter.chars().count()),
            Some(IdentityTokensViewPrefs::MAX_FILTER_CHARS)
        );
        let same = state.prefs.clone().expect("edited");
        state.prefs_dirty = false;
        state.edit_prefs(same);
        assert!(!state.prefs_dirty(), "an unchanged edit is not dirty");
    }

    /// A token cache notification re-reads the rows on the next frame, from
    /// this tab's task or another screen's; other domains leave them alone.
    #[test]
    fn a_token_cache_update_rereads_the_rows() {
        use crate::backend_task::CacheDomain;
        let mut state = IdentityTokensState::default();
        for (domain, rereads) in [
            (CacheDomain::Tokens, true),
            (CacheDomain::Identities, true),
            (CacheDomain::All, true),
            (CacheDomain::ContactPayments, false),
            (CacheDomain::Dpns, false),
        ] {
            state.rows_read_at = Some(Instant::now());
            state.cached_data_changed(domain);
            assert_eq!(state.rows_read_at.is_none(), rereads, "{domain:?}");
        }
    }
}
