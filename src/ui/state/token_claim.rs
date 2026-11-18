//! Request ownership for the identity hub's claim dialog. Renders nothing.
use crate::app::AppAction;
use crate::backend_task::error::TaskError;
use crate::backend_task::tokens::TokenTask;
use crate::backend_task::{BackendTask, BackendTaskContext, BackendTaskSuccessResult};
use crate::context::AppContext;
use crate::model::qualified_identity::QualifiedIdentity;
use crate::model::token::{IdentityTokenIdentifier, claimable_distributions, next_scheduled_claim};
use crate::ui::MessageType;
use crate::ui::components::MessageBanner;
use crate::ui::identity::token_claim::{ClaimDecision, TokenClaimDialog};
use crate::ui::state::PendingRequest;
use crate::ui::state::identity_tokens::TokenRow;
use crate::ui::tokens::task_feedback;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use dash_sdk::platform::DataContract;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct ClaimForm {
    row: TokenRow,
    identity: QualifiedIdentity,
    contract: Arc<DataContract>,
}

/// A claim stays owned across identity/tab changes; its form does not.
#[derive(Default)]
pub struct TokenClaimState {
    pub dialog: Option<TokenClaimDialog>,
    form: Option<ClaimForm>,
    estimate: PendingRequest<IdentityTokenIdentifier>,
    estimate_owner: Option<egui::Context>,
    estimates: BTreeMap<IdentityTokenIdentifier, (u64, Instant)>,
}

impl TokenClaimState {
    /// Distributions eligible for this identity, excluding known empty rewards.
    pub fn offered(
        &self,
        app_context: &AppContext,
        row: &TokenRow,
        identity: &QualifiedIdentity,
    ) -> Vec<TokenDistributionType> {
        let receipt = task_feedback::receipt(app_context, row.pair());
        let once_claimed = receipt.once
            || row
                .once_claim
                .as_ref()
                .is_some_and(|once| once.claimed_at_ms.is_some());
        let mut offered = claimable_distributions(
            &row.token.token_config,
            row.issuer_id,
            identity,
            once_claimed,
            receipt.scheduled_through,
            crate::utils::time::now_ms(),
        );
        if self
            .estimates
            .get(&row.pair())
            .is_some_and(|(amount, at)| *amount == 0 && at.elapsed() < Duration::from_secs(120))
        {
            offered.retain(|kind| *kind != TokenDistributionType::Perpetual);
        }
        offered
    }

    /// A paid claim is outstanding; no second claim can be submitted.
    pub fn is_pending(&self, app_context: &AppContext) -> bool {
        task_feedback::pending(app_context, None)
    }

    /// Only this row is claiming; other rows may be disabled but are not in flight.
    pub fn is_pending_for(&self, app_context: &AppContext, pair: IdentityTokenIdentifier) -> bool {
        task_feedback::pending(app_context, Some(pair))
    }

    pub fn owns_estimate(&self, context: &BackendTaskContext) -> bool {
        self.estimate.owns(context)
    }

    /// Close a form without canceling or replacing a submitted operation.
    pub fn close(&mut self) {
        self.dialog = None;
        self.form = None;
        if let Some(owner) = self.estimate_owner.take()
            && let Some(context) = self.estimate.context()
        {
            task_feedback::abandon_estimate(&owner, context);
        }
        self.estimate.clear();
    }

    /// Open a form using the hub's identity and token, never the legacy Tokens screen.
    pub fn open(
        &mut self,
        app_context: &Arc<AppContext>,
        identity: &QualifiedIdentity,
        row: &TokenRow,
    ) -> AppAction {
        let offered = self.offered(app_context, row, identity);
        if self.is_pending(app_context) || offered.is_empty() {
            return AppAction::None;
        }
        let contract = match app_context.get_contract_by_id(&row.token.data_contract_id) {
            Ok(Some(contract)) => contract.contract,
            result => {
                let error = result.err().unwrap_or(TaskError::DataContractNotFound);
                MessageBanner::set_global(
                    app_context.egui_ctx(),
                    "The token could not be opened. Refresh the token list and try again.",
                    MessageType::Error,
                )
                .with_details(error);
                return AppAction::None;
            }
        };
        self.close();
        let estimate = offered.contains(&TokenDistributionType::Perpetual);
        self.dialog = Some(TokenClaimDialog::new(app_context, identity, row, offered));
        self.form = Some(ClaimForm {
            row: row.clone(),
            identity: identity.clone(),
            contract: Arc::new(contract),
        });
        if estimate {
            let action = self.estimate.dispatch_on_with(
                BackendTask::TokenTask(Box::new(
                    TokenTask::EstimatePerpetualTokenRewardsWithExplanation {
                        identity_id: row.pair().identity_id,
                        token_id: row.pair().token_id,
                    },
                )),
                app_context.network,
                row.pair(),
            );
            if let Some(context) = self.estimate.context() {
                task_feedback::track_estimate(app_context.egui_ctx(), context.clone());
                self.estimate_owner = Some(app_context.egui_ctx().clone());
            }
            action
        } else {
            AppAction::None
        }
    }

    /// Dispatch exactly the key and distribution approved by the confirmation.
    pub fn decide(&mut self, app_context: &Arc<AppContext>, decision: ClaimDecision) -> AppAction {
        let ClaimDecision::Confirmed(distribution_type, signing_key) = decision else {
            self.close();
            return AppAction::None;
        };
        let Some(form) = self.form.take() else {
            return AppAction::None;
        };
        self.close();
        if self.is_pending(app_context) {
            return AppAction::None;
        }
        let scheduled_moment = next_scheduled_claim(
            &form.row.token.token_config,
            form.row.pair().identity_id,
            task_feedback::receipt(app_context, form.row.pair()).scheduled_through,
            crate::utils::time::now_ms(),
        );
        let task = BackendTask::TokenTask(Box::new(TokenTask::ClaimTokens {
            data_contract: form.contract,
            token_position: form.row.token.token_position,
            actor_identity: form.identity,
            distribution_type,
            signing_key,
            public_note: None,
        }));
        let context = BackendTaskContext::for_dispatch_on(&task, app_context.network);
        task_feedback::track(
            app_context,
            context.clone(),
            &form.row,
            distribution_type,
            scheduled_moment,
        );
        AppAction::BackendTaskWithContext { task, context }
    }

    /// Returns true only for this dialog's own request.
    pub fn record_result(
        &mut self,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> bool {
        if let Some(pair) = self.estimate.subject_for(context).copied() {
            if let BackendTaskSuccessResult::TokenEstimatedNonClaimedPerpetualDistributionAmountWithExplanation(result_pair, amount, _) = result
                && *result_pair == pair
            {
                self.estimate.settle(context);
                if let Some(owner) = self.estimate_owner.take() {
                    task_feedback::finish_estimate(&owner, context, false);
                }
                self.estimates.insert(pair, (*amount, Instant::now()));
                if let Some(dialog) = &mut self.dialog { dialog.estimated(Some(*amount)); }
            }
            return true;
        }
        false
    }

    /// Errors still reach the app's typed-error banner; no paid automatic retry.
    pub fn record_error(&mut self, context: &BackendTaskContext, _error: &TaskError) {
        if self.estimate.settle(context).is_some() {
            if let Some(owner) = self.estimate_owner.take() {
                if matches!(_error, TaskError::NetworkSwitchedBeforeResult) {
                    task_feedback::abandon_estimate(&owner, context);
                } else {
                    task_feedback::finish_estimate(&owner, context, false);
                }
            }
            if let Some(dialog) = &mut self.dialog {
                dialog.estimated(None);
            }
        }
    }
}

impl Drop for TokenClaimState {
    fn drop(&mut self) {
        self.close();
    }
}
