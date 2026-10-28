//! Identity Hub Tokens tab state: the selected identity's token rows, the
//! balance refresh it waits on, and the list controls remembered per device.
//!
//! Renders nothing. Rows are derived from the token registry, the per-pair
//! dismissals and the upstream balance snapshot, all read through
//! `AppContext`; the derivation is cached for a few seconds because the
//! snapshot changes without telling the UI.

use crate::app::AppAction;
use crate::backend_task::error::TaskError;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::amount::Amount;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::token::{
    IdentityTokenBalance, IdentityTokenBasicInfo, IdentityTokenIdentifier, IdentityTokensViewPrefs,
    TokenListSort, get_available_token_actions_for_identity, identity_has_token_admin_permission,
    once_per_identity_amount,
};
use crate::ui::components::confirmation_dialog::ConfirmationDialog;
use crate::ui::state::PendingRequest;
use dash_sdk::dpp::balances::credits::TokenAmount;
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::TokenConfiguration;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v0::TokenConfigurationV0Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v1::TokenConfigurationV1Getters;
use dash_sdk::dpp::data_contract::associated_token::token_configuration_convention::accessors::v0::TokenConfigurationConventionV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::string_encoding::Encoding;
use dash_sdk::platform::Identifier;
use std::collections::BTreeSet;
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

/// One tracked token of the selected identity.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenRow {
    pub pair: IdentityTokenIdentifier,
    /// The registry name, also used as the balance unit.
    pub name: String,
    pub contract_id: Identifier,
    pub token_position: u16,
    pub token_config: TokenConfiguration,
    /// `None` until the identity's balance of this token has been synced.
    pub balance: Option<TokenAmount>,
    /// The identity issued the token (owns its contract).
    pub issued_by_me: bool,
    /// Holders may send this token to another identity.
    pub transferable: bool,
    /// A distribution this identity may claim, before once-per-identity bookkeeping.
    pub can_claim: bool,
    pub once_claim: Option<OnceClaim>,
    /// The identity holds an administrative permission on the token.
    pub has_admin_permission: bool,
    /// The last balance check of this pair failed and no balance is known.
    pub check_failed: bool,
}

impl TokenRow {
    /// The balance in the token's decimals, with the name as unit.
    pub fn amount(&self, value: TokenAmount) -> Amount {
        Amount::new(value, self.token_config.conventions().decimals()).with_unit_name(&self.name)
    }

    /// The balance as the Send screen expects it, once known.
    pub fn balance_record(&self) -> Option<IdentityTokenBalance> {
        Some(IdentityTokenBalance {
            token_id: self.pair.token_id,
            token_alias: self.name.clone(),
            token_config: self.token_config.clone(),
            identity_id: self.pair.identity_id,
            balance: self.balance?,
            estimated_unclaimed_rewards: None,
            data_contract_id: self.contract_id,
            token_position: self.token_position,
        })
    }

    /// The handles the Claim screen expects.
    pub fn basic_info(&self) -> IdentityTokenBasicInfo {
        IdentityTokenBasicInfo {
            token_id: self.pair.token_id,
            token_alias: self.name.clone(),
            identity_id: self.pair.identity_id,
            contract_id: self.contract_id,
            token_position: self.token_position,
        }
    }

    /// The token ID as users copy and compare it.
    pub fn token_id_base58(&self) -> String {
        self.pair.token_id.to_string(Encoding::Base58)
    }

    /// A claim the identity can still take: a distribution it may claim,
    /// unless its once-per-identity claim is already taken.
    pub fn claim_available(&self) -> bool {
        self.can_claim
            && !self
                .once_claim
                .as_ref()
                .is_some_and(|once| once.claimed_at_ms.is_some())
    }

    /// The identity holds some of this token.
    pub fn holds_some(&self) -> bool {
        self.balance.is_some_and(|balance| balance > 0)
    }

    /// Whether the name, symbol or ID contains `filter`, case-insensitively.
    /// An empty filter matches everything.
    pub fn matches_filter(&self, filter: &str) -> bool {
        let filter = filter.trim().to_lowercase();
        if filter.is_empty() {
            return true;
        }
        self.name.to_lowercase().contains(&filter)
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
    /// The last check of this pair failed and no balance is known.
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
    if row.balance.is_none() {
        return if row.check_failed {
            RowStatus::CouldNotCheck
        } else {
            RowStatus::Checking
        };
    }
    if view_only && (row.claim_available() || (row.holds_some() && row.transferable)) {
        return RowStatus::AddKey;
    }
    if let Some(once) = &row.once_claim {
        return match once.claimed_at_ms {
            Some(claimed_at_ms) => RowStatus::ClaimedOn(
                crate::model::datetime::instant_from_unix_millis(claimed_at_ms)
                    .map(crate::model::datetime::local_date)
                    .unwrap_or_default(),
            ),
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
pub fn sort_rows(rows: &mut [TokenRow], sort: TokenListSort) {
    rows.sort_by(|a, b| {
        let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
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
    match row.balance {
        Some(0) => (0, 0),
        None => (1, 0),
        Some(balance) => (2, balance),
    }
}

/// Whether `row` stays listed under the current controls. The hide-zero
/// control hides only a fetched zero, never an unknown balance.
pub fn row_is_listed(row: &TokenRow, prefs: &IdentityTokensViewPrefs) -> bool {
    if prefs.hide_zero_balances && row.balance == Some(0) {
        return false;
    }
    row.matches_filter(&prefs.filter)
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
            TaskError::TokenBalanceRefreshTimeout { .. } => RefreshFailure::TooSlow,
            _ => RefreshFailure::Failed,
        }
    }
}

/// What the Tokens tab shows and waits on for the selected identity.
#[derive(Default)]
pub struct IdentityTokensState {
    owner: Option<(Network, Identifier)>,
    rows: Vec<TokenRow>,
    rows_read_at: Option<Instant>,
    /// Tokens exist in the registry even when every row is dismissed.
    registry_has_tokens: bool,
    /// Pairs whose last explicit check failed with no balance to show.
    failed_checks: BTreeSet<IdentityTokenIdentifier>,
    refresh: PendingRequest,
    pair_check: PendingRequest<IdentityTokenIdentifier>,
    stop_tracking: PendingRequest<IdentityTokenIdentifier>,
    last_refreshed_at_ms: Option<u64>,
    refresh_failure: Option<RefreshFailure>,
    arriving: bool,
    /// Set when the error just recorded belongs to a request of this tab, so
    /// the hub shows no global banner for it.
    swallow_next_error: bool,
    /// A task of this tab succeeded; its screen-wide refresh must keep the
    /// hub's other content on screen instead of blanking it.
    expects_screen_refresh: bool,
    prefs: Option<IdentityTokensViewPrefs>,
    /// The Stop tracking confirmation, with the pair it is about.
    pub stop_tracking_dialog: Option<(ConfirmationDialog, IdentityTokenIdentifier)>,
    /// The token whose detail page is open instead of the list.
    pub detail: Option<IdentityTokenIdentifier>,
    /// The refresh error banner was dismissed; rows keep their stale time.
    pub failure_dismissed: bool,
}

impl IdentityTokensState {
    /// Drop everything that belongs to the previous identity. Device
    /// preferences and an in-flight request's bookkeeping survive.
    pub fn reset_for_identity_change(&mut self) {
        let prefs = self.prefs.take();
        let expects_screen_refresh = self.expects_screen_refresh;
        *self = Self {
            prefs,
            expects_screen_refresh,
            arriving: true,
            ..Self::default()
        };
    }

    /// Bind the rows to the identity on screen, dropping another identity's.
    fn reconcile_owner(&mut self, identity: &QualifiedIdentity) {
        let owner = Some((identity.network, identity.identity.id()));
        if self.owner != owner {
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

    /// The identity's rows, re-read when stale or invalidated.
    pub fn rows(&mut self, app_context: &AppContext, identity: &QualifiedIdentity) -> &[TokenRow] {
        self.reconcile_owner(identity);
        let stale = self
            .rows_read_at
            .is_none_or(|read_at| read_at.elapsed() >= ROWS_TTL);
        if stale {
            match self.read_rows(app_context, identity) {
                Ok((rows, registry_has_tokens)) => {
                    self.rows = rows;
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
        &self.rows
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
            rows.push(TokenRow {
                pair,
                name: token.token_name,
                contract_id: token.data_contract.id(),
                token_position: token.token_position,
                issued_by_me: token.data_contract.owner_id() == identity_id,
                transferable: config.is_transferable(),
                can_claim: actions.can_claim,
                has_admin_permission: identity_has_token_admin_permission(
                    identity,
                    config,
                    &token.data_contract,
                ),
                check_failed: balance.is_none() && self.failed_checks.contains(&pair),
                once_claim,
                token_config: token.token_configuration,
                balance,
            });
        }
        Ok((rows, registry_has_tokens))
    }

    /// Tokens are known on this device, whether or not this identity tracks them.
    pub fn registry_has_tokens(&self) -> bool {
        self.registry_has_tokens
    }

    /// How many of the identity's tokens have something to claim.
    pub fn pending_claims(&self) -> usize {
        self.rows.iter().filter(|row| row.claim_available()).count()
    }

    /// The row of `pair`, if the identity tracks it.
    pub fn row(&self, pair: &IdentityTokenIdentifier) -> Option<&TokenRow> {
        self.rows.iter().find(|row| row.pair == *pair)
    }

    /// The list controls, read from the device on first use.
    pub fn prefs(&mut self, app_context: &AppContext) -> &IdentityTokensViewPrefs {
        self.prefs
            .get_or_insert_with(|| app_context.identity_tokens_view_prefs())
    }

    /// Change the list controls and remember them on this device.
    pub fn set_prefs(&mut self, app_context: &AppContext, prefs: IdentityTokensViewPrefs) {
        if let Err(error) = app_context.save_identity_tokens_view_prefs(&prefs) {
            tracing::warn!(
                error = ?error,
                "Could not save the Identity Hub token list preferences; keeping them for this session"
            );
        }
        self.prefs = Some(prefs);
    }

    /// When balances were last refreshed at this tab's request (Unix ms).
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
        self.refresh.is_pending()
    }

    /// The hub-wide refresh that follows a token task of this tab arrived;
    /// answer once whether the hub should keep its content on screen.
    pub fn take_expected_screen_refresh(&mut self) -> bool {
        std::mem::take(&mut self.expects_screen_refresh)
    }

    /// The arrival refresh, when due. Claims the hub's dispatch slot, so the
    /// caller asks only on a frame nothing else produced an action.
    pub fn load_action(&mut self, app_context: &AppContext, now_ms: u64) -> AppAction {
        if !arrival_refresh_due(self.arriving, now_ms, self.last_refreshed_at_ms) {
            self.arriving = false;
            return AppAction::None;
        }
        // Proof-verifying reads sent before the masternode list is synced fail
        // and get the queried nodes banned, so the arrival stays owed.
        if !app_context.connection_status().masternodes_ready() {
            return AppAction::None;
        }
        self.arriving = false;
        self.refresh_action(app_context)
    }

    /// Ask for a balance refresh now, unless one is already running anywhere
    /// in the app — the guard is shared with My Tokens.
    pub fn refresh_action(&mut self, app_context: &AppContext) -> AppAction {
        if self.refresh.is_pending() || app_context.token_balance_refresh_in_flight() {
            return AppAction::None;
        }
        self.failure_dismissed = false;
        self.refresh.dispatch(BackendTask::TokenTask(Box::new(
            TokenTask::QueryMyTokenBalances,
        )))
    }

    /// Check one pair's balance now; also tracks the pair again.
    pub fn check_pair_action(&mut self, pair: IdentityTokenIdentifier) -> AppAction {
        self.pair_check.dispatch_with(
            BackendTask::TokenTask(Box::new(TokenTask::QueryIdentityTokenBalance(pair))),
            pair,
        )
    }

    /// Stop tracking `pair` for this identity; its row leaves at once.
    pub fn stop_tracking_action(&mut self, pair: IdentityTokenIdentifier) -> AppAction {
        self.rows.retain(|row| row.pair != pair);
        if self.detail == Some(pair) {
            self.detail = None;
        }
        self.stop_tracking.dispatch_with(
            BackendTask::TokenTask(Box::new(TokenTask::StopTrackingTokenBalance(pair))),
            pair,
        )
    }

    /// Track every dismissed token of `identity` again by checking its balance.
    pub fn track_again_action(
        &mut self,
        app_context: &AppContext,
        identity: &QualifiedIdentity,
    ) -> AppAction {
        let identity_id = identity.identity.id();
        let dismissed = match app_context.untracked_token_balances() {
            Ok(dismissed) => dismissed,
            Err(error) => {
                tracing::warn!(error = ?error, "Could not read the dismissed token pairs");
                return AppAction::None;
            }
        };
        let tasks: Vec<BackendTask> = dismissed
            .into_iter()
            .filter(|pair| pair.identity_id == identity_id)
            .map(|pair| {
                BackendTask::TokenTask(Box::new(TokenTask::QueryIdentityTokenBalance(pair)))
            })
            .collect();
        if tasks.is_empty() {
            return AppAction::None;
        }
        self.expects_screen_refresh = true;
        self.invalidate_rows();
        AppAction::BackendTasks(tasks, crate::app::BackendTasksExecutionMode::Sequential)
    }

    /// Settle a result of this tab's requests. Returns whether it was one.
    pub fn record_result(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
        now_ms: u64,
    ) -> bool {
        if self.refresh.settle(context).is_some() {
            if matches!(result, BackendTaskSuccessResult::FetchedTokenBalances) {
                self.last_refreshed_at_ms = Some(now_ms);
                self.refresh_failure = None;
                self.expects_screen_refresh = true;
            }
            self.invalidate_rows();
            return true;
        }
        if let Some(pair) = self.pair_check.settle(context) {
            self.failed_checks.remove(&pair);
            self.expects_screen_refresh = true;
            self.invalidate_rows();
            return true;
        }
        if self.stop_tracking.settle(context).is_some() {
            self.expects_screen_refresh = true;
            self.invalidate_rows();
            return true;
        }
        false
    }

    /// Note a failure of this tab's requests. A refresh already running
    /// elsewhere is not a failure; anything else is shown inline, never as a
    /// global banner.
    pub fn record_error(&mut self, context: &BackendTaskContext, error: &TaskError) {
        if self.refresh.settle(context).is_some() {
            if !matches!(error, TaskError::TokenBalanceRefreshInProgress) {
                tracing::warn!(error = ?error, "The Identity Hub token balance refresh failed");
                self.refresh_failure = Some(RefreshFailure::from_error(error));
                self.failure_dismissed = false;
            }
            self.swallow_next_error = true;
            return;
        }
        if let Some(pair) = self.pair_check.settle(context) {
            if !matches!(error, TaskError::TokenBalanceRefreshInProgress) {
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

    /// Whether the error just recorded is shown by this tab rather than a banner.
    pub fn take_swallowed_error(&mut self) -> bool {
        std::mem::take(&mut self.swallow_next_error)
    }

    /// The hub's token rows changed under another screen's task.
    pub fn balances_changed(&mut self) {
        self.invalidate_rows();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::accessors::v1::TokenConfigurationV1Setters;
    use dash_sdk::dpp::data_contract::associated_token::token_configuration::v0::TokenConfigurationV0;

    fn row(name: &str, balance: Option<TokenAmount>) -> TokenRow {
        TokenRow {
            pair: IdentityTokenIdentifier {
                identity_id: Identifier::from([1; 32]),
                token_id: Identifier::from([name.len() as u8; 32]),
            },
            name: name.into(),
            contract_id: Identifier::from([9; 32]),
            token_position: 0,
            token_config: TokenConfiguration::V0(TokenConfigurationV0::default_most_restrictive()),
            balance,
            issued_by_me: false,
            transferable: true,
            can_claim: false,
            once_claim: None,
            has_admin_permission: false,
            check_failed: false,
        }
    }

    fn names(rows: &[TokenRow]) -> Vec<&str> {
        rows.iter().map(|row| row.name.as_str()).collect()
    }

    #[test]
    fn claimable_first_then_balance_with_unknown_before_zero() {
        let mut claimable = row("Claimable", Some(0));
        claimable.can_claim = true;
        let mut rows = vec![
            row("Zero", Some(0)),
            row("Small", Some(1)),
            row("Checking", None),
            row("Big", Some(1_000)),
            claimable,
        ];
        sort_rows(&mut rows, TokenListSort::ClaimableFirst);
        assert_eq!(
            names(&rows),
            ["Claimable", "Big", "Small", "Checking", "Zero"]
        );
    }

    #[test]
    fn a_taken_once_claim_is_no_longer_claimable_and_sorts_by_balance() {
        let mut taken = row("Taken", Some(100));
        taken.can_claim = true;
        taken.once_claim = Some(OnceClaim {
            amount: 100,
            claimed_at_ms: Some(1),
        });
        let mut rows = vec![taken, row("Bigger", Some(200))];
        sort_rows(&mut rows, TokenListSort::ClaimableFirst);
        assert_eq!(names(&rows), ["Bigger", "Taken"]);
    }

    #[test]
    fn balance_and_name_sorts() {
        let mut rows = vec![row("b", Some(1)), row("A", None), row("c", Some(5))];
        sort_rows(&mut rows, TokenListSort::BalanceHighToLow);
        assert_eq!(names(&rows), ["c", "b", "A"]);
        sort_rows(&mut rows, TokenListSort::NameAToZ);
        assert_eq!(names(&rows), ["A", "b", "c"]);
    }

    #[test]
    fn hide_zero_hides_a_fetched_zero_but_never_an_unknown_balance() {
        let prefs = IdentityTokensViewPrefs {
            hide_zero_balances: true,
            ..Default::default()
        };
        assert!(!row_is_listed(&row("Zero", Some(0)), &prefs));
        assert!(row_is_listed(&row("Checking", None), &prefs));
        assert!(row_is_listed(&row("Held", Some(3)), &prefs));
        assert!(row_is_listed(
            &row("Zero", Some(0)),
            &IdentityTokensViewPrefs::default()
        ));
    }

    #[test]
    fn filter_matches_name_or_id_case_insensitively() {
        let prefs = |filter: &str| IdentityTokensViewPrefs {
            filter: filter.into(),
            ..Default::default()
        };
        let coffee = row("DashCoffee", Some(1));
        assert!(row_is_listed(&coffee, &prefs("coffee")));
        assert!(row_is_listed(&coffee, &prefs("  ")));
        assert!(row_is_listed(
            &coffee,
            &prefs(&coffee.token_id_base58()[..6].to_lowercase())
        ));
        assert!(!row_is_listed(&coffee, &prefs("points")));
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

        let mut once = row("AIR", Some(0));
        once.can_claim = true;
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

        let mut rewards = row("DCF", Some(5));
        rewards.can_claim = true;
        assert_eq!(row_status(&rewards, false), RowStatus::RewardsMayBeWaiting);
    }

    #[test]
    fn a_non_transferable_zero_row_needs_no_key() {
        let mut badge = row("FAN", Some(1));
        badge.set_non_transferable();
        assert_eq!(row_status(&badge, true), RowStatus::None);
    }

    impl TokenRow {
        fn set_non_transferable(&mut self) {
            self.transferable = false;
            self.token_config.set_transferable(false);
        }
    }

    #[test]
    fn arrival_refresh_is_due_when_never_or_old() {
        let two_minutes = ARRIVAL_REFRESH_AFTER.as_millis() as u64;
        assert!(arrival_refresh_due(true, 10_000, None));
        assert!(arrival_refresh_due(true, two_minutes + 5, Some(5)));
        assert!(!arrival_refresh_due(true, two_minutes, Some(5)));
        assert!(!arrival_refresh_due(false, 10_000, None));
    }

    #[test]
    fn pending_claims_counts_claimable_rows_only() {
        let mut state = IdentityTokensState::default();
        let mut claimable = row("A", Some(0));
        claimable.can_claim = true;
        let mut taken = row("B", Some(0));
        taken.can_claim = true;
        taken.once_claim = Some(OnceClaim {
            amount: 1,
            claimed_at_ms: Some(1),
        });
        state.rows = vec![claimable, taken, row("C", Some(4))];
        assert_eq!(state.pending_claims(), 1);
    }

    #[test]
    fn a_refresh_result_records_the_time_and_keeps_the_hub_content() {
        let mut state = IdentityTokensState::default();
        let task = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));
        let AppAction::BackendTaskWithContext { context, .. } = state.refresh.dispatch(task) else {
            panic!("a refresh dispatches with its own context");
        };
        assert!(state.refresh_pending());
        assert!(!state.record_result(
            &BackendTaskContext::TokenBalanceRefresh,
            &BackendTaskSuccessResult::FetchedTokenBalances,
            7,
        ));
        assert!(state.record_result(&context, &BackendTaskSuccessResult::FetchedTokenBalances, 7,));
        assert_eq!(state.last_refreshed_at_ms(), Some(7));
        assert!(!state.refresh_pending());
        assert!(state.take_expected_screen_refresh());
        assert!(!state.take_expected_screen_refresh());
    }

    #[test]
    fn an_in_progress_refresh_is_not_a_failure_but_a_timeout_is() {
        let mut state = IdentityTokensState::default();
        let task = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));
        let AppAction::BackendTaskWithContext { context, .. } = state.refresh.dispatch(task) else {
            panic!("a refresh dispatches with its own context");
        };
        state.record_error(&context, &TaskError::TokenBalanceRefreshInProgress);
        assert_eq!(state.refresh_failure(), None);
        assert!(state.take_swallowed_error());

        let task = BackendTask::TokenTask(Box::new(TokenTask::QueryMyTokenBalances));
        let AppAction::BackendTaskWithContext { context, .. } = state.refresh.dispatch(task) else {
            panic!("a refresh dispatches with its own context");
        };
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

    #[test]
    fn a_failed_pair_check_marks_the_row_until_it_succeeds() {
        let mut state = IdentityTokensState::default();
        let pair = row("T", None).pair;
        let AppAction::BackendTaskWithContext { context, .. } = state.check_pair_action(pair)
        else {
            panic!("a pair check dispatches with its own context");
        };
        state.record_error(&context, &TaskError::NoIdentitiesFound);
        assert!(state.failed_checks.contains(&pair));
        assert!(state.take_swallowed_error());

        let AppAction::BackendTaskWithContext { context, .. } = state.check_pair_action(pair)
        else {
            panic!("a pair check dispatches with its own context");
        };
        assert!(state.record_result(&context, &BackendTaskSuccessResult::FetchedTokenBalances, 1));
        assert!(!state.failed_checks.contains(&pair));
    }

    #[test]
    fn stop_tracking_removes_the_row_at_once() {
        let mut state = IdentityTokensState {
            rows: vec![row("A", Some(1)), row("Bee", Some(2))],
            ..Default::default()
        };
        let pair = state.rows[0].pair;
        state.detail = Some(pair);
        let _ = state.stop_tracking_action(pair);
        assert_eq!(names(&state.rows), ["Bee"]);
        assert_eq!(state.detail, None);
    }
}
