//! Claim ownership survives screen disposal and network switches, like identity top-ups.
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheDomain, CacheUpdate};
use crate::context::AppContext;
use crate::model::token::IdentityTokenIdentifier;
use crate::ui::MessageType;
use crate::ui::components::{BannerHandle, MessageBanner};
use crate::ui::state::identity_tokens::TokenRow;
use crate::ui::task_feedback::{DomainFeedback, Feedback};
use dash_sdk::dpp::dashcore::Network;
use dash_sdk::dpp::data_contract::associated_token::token_distribution_key::TokenDistributionType;
use std::collections::{BTreeMap, BTreeSet};

const CLAIMS_ID: &str = "identity_hub_token_claims";

#[derive(Clone)]
struct ClaimInFlight {
    context: BackendTaskContext,
    network: Network,
    pair: IdentityTokenIdentifier,
    distribution: TokenDistributionType,
    scheduled_moment: Option<u64>,
    token_name: String,
    progress: BannerHandle,
}

/// Confirmed local claims only; an unproven refusal never consumes an entitlement.
#[derive(Clone, Default)]
pub(crate) struct ClaimReceipt {
    pub once: bool,
    /// A lower bound: other devices may have already claimed later installments.
    pub scheduled_through: Option<u64>,
}

#[derive(Clone, Default)]
struct Claims {
    pending: Vec<ClaimInFlight>,
    receipts: BTreeMap<(Network, IdentityTokenIdentifier), ClaimReceipt>,
    refresh: BTreeSet<Network>,
    estimates: Vec<(BackendTaskContext, bool)>,
}

fn with_claims<R>(ctx: &egui::Context, f: impl FnOnce(&mut Claims) -> R) -> R {
    ctx.data_mut(|data| f(data.get_temp_mut_or_default(egui::Id::new(CLAIMS_ID))))
}

#[cfg(test)]
fn claims(ctx: &egui::Context) -> Claims {
    with_claims(ctx, |state| state.clone())
}

#[cfg(test)]
fn store(ctx: &egui::Context, claims: Claims) {
    with_claims(ctx, |state| *state = claims);
}

pub(crate) fn receipt(app_context: &AppContext, pair: IdentityTokenIdentifier) -> ClaimReceipt {
    with_claims(app_context.egui_ctx(), |state| {
        state
            .receipts
            .get(&(app_context.network, pair))
            .cloned()
            .unwrap_or_default()
    })
}

pub(crate) fn pending(app_context: &AppContext, pair: Option<IdentityTokenIdentifier>) -> bool {
    with_claims(app_context.egui_ctx(), |state| {
        state.pending.iter().any(|claim| {
            claim.network == app_context.network && pair.is_none_or(|pair| claim.pair == pair)
        })
    })
}

pub(crate) fn refresh_owed(app_context: &AppContext) -> bool {
    with_claims(app_context.egui_ctx(), |state| {
        state.refresh.contains(&app_context.network)
    })
}

pub(crate) fn refresh_dispatched(app_context: &AppContext) {
    with_claims(app_context.egui_ctx(), |state| {
        state.refresh.remove(&app_context.network);
    });
}

pub(crate) fn track_estimate(ctx: &egui::Context, context: BackendTaskContext) {
    with_claims(ctx, |state| state.estimates.push((context, false)));
}

pub(crate) fn abandon_estimate(ctx: &egui::Context, context: &BackendTaskContext) {
    with_claims(ctx, |state| {
        if let Some((_, abandoned)) = state.estimates.iter_mut().find(|(own, _)| own == context) {
            *abandoned = true;
        }
    });
}

/// Only hub estimates are quiet after their form disappears; paid errors remain visible.
pub(crate) fn finish_estimate(
    ctx: &egui::Context,
    context: &BackendTaskContext,
    sender_gone: bool,
) -> Option<bool> {
    with_claims(ctx, |state| {
        let index = state.estimates.iter().position(|(own, _)| own == context)?;
        let (_, abandoned) = state.estimates.remove(index);
        Some(abandoned || sender_gone)
    })
}

/// Called on the UI thread before returning the paid task to the dispatcher.
pub(crate) fn track(
    app_context: &AppContext,
    context: BackendTaskContext,
    row: &TokenRow,
    distribution: TokenDistributionType,
    scheduled_moment: Option<u64>,
) {
    let network = crate::ui::network_chooser_screen::chooser_network_label(app_context.network);
    let token = row.name();
    let progress = MessageBanner::set_global(
        app_context.egui_ctx(),
        format!("Claiming {token} rewards on {network}…"),
        MessageType::Info,
    );
    progress.with_elapsed();
    with_claims(app_context.egui_ctx(), |state| {
        state.pending.push(ClaimInFlight {
            context,
            network: app_context.network,
            pair: row.pair(),
            distribution,
            scheduled_moment,
            token_name: token.to_owned(),
            progress,
        })
    });
}

fn finish(
    ctx: &egui::Context,
    context: &BackendTaskContext,
    succeeded: bool,
    notify: bool,
) -> Option<Network> {
    let claim = with_claims(ctx, |state| {
        let index = state
            .pending
            .iter()
            .position(|claim| claim.context == *context)?;
        let claim = state.pending.remove(index);
        if succeeded {
            match claim.distribution {
                TokenDistributionType::OncePerIdentity => {
                    state
                        .receipts
                        .entry((claim.network, claim.pair))
                        .or_default()
                        .once = true;
                }
                TokenDistributionType::PreProgrammed => {
                    let receipt = state
                        .receipts
                        .entry((claim.network, claim.pair))
                        .or_default();
                    receipt.scheduled_through =
                        receipt.scheduled_through.max(claim.scheduled_moment);
                }
                TokenDistributionType::Perpetual => {}
            }
            state.refresh.insert(claim.network);
        }
        Some(claim)
    })?;
    // Banner methods also lock egui data, so call them outside with_claims.
    claim.progress.clear();
    if succeeded && notify {
        let network = crate::ui::network_chooser_screen::chooser_network_label(claim.network);
        let identity = claim.pair.identity_id;
        let token = claim.token_name;
        MessageBanner::set_global(
            ctx,
            format!(
                "Claimed {token} rewards for identity {identity} on {network}. Open that network's token list to check the balance."
            ),
            MessageType::Success,
        );
    }
    Some(claim.network)
}

/// Idempotent settlement: also used by direct screen deliveries in isolated UI tests.
pub(crate) fn record_success(
    ctx: &egui::Context,
    context: &BackendTaskContext,
    result: &BackendTaskSuccessResult,
) -> Option<Network> {
    finish_estimate(ctx, context, false);
    if matches!(result, BackendTaskSuccessResult::ClaimedTokens(_)) {
        finish(ctx, context, true, true)
    } else {
        None
    }
}

pub(crate) fn record_error(ctx: &egui::Context, context: &BackendTaskContext, error: &TaskError) {
    // This is an end-of-wait notification, not the task's actual outcome.
    // DomainFeedback receives the real outcome even on an inactive network.
    if matches!(error, TaskError::NetworkSwitchedBeforeResult) {
        return;
    }
    finish(
        ctx,
        context,
        matches!(error, TaskError::TokensClaimedButClaimNotRecorded { .. }),
        false,
    );
}

pub(crate) struct TokensFeedback;

impl DomainFeedback for TokensFeedback {
    fn on_success(
        &self,
        feedback: &Feedback<'_>,
        context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> Vec<CacheUpdate> {
        if let Some(network) = record_success(feedback.ctx, context, result) {
            return vec![CacheUpdate {
                network,
                domain: CacheDomain::Tokens,
            }];
        }
        if matches!(result, BackendTaskSuccessResult::ClaimedTokens(_)) {
            MessageBanner::set_global(
                feedback.ctx,
                "Token rewards were claimed. Open the token list on the network used for the claim to check the balance.",
                MessageType::Success,
            );
        }
        Vec::new()
    }

    fn on_error(
        &self,
        feedback: &Feedback<'_>,
        context: &BackendTaskContext,
        error: &TaskError,
    ) -> bool {
        record_error(feedback.ctx, context, error);
        if matches!(error, TaskError::NetworkSwitchedBeforeResult) {
            return false;
        }
        finish_estimate(feedback.ctx, context, feedback.delivery.sender_gone).unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend_task::tokens::TokenTask;
    use crate::backend_task::{BackendTask, FeeResult};
    use crate::ui::task_feedback::ResultDelivery;
    use dash_sdk::platform::Identifier;

    fn begin(
        ctx: &egui::Context,
        network: Network,
        distribution: TokenDistributionType,
        at: Option<u64>,
    ) -> (BackendTaskContext, IdentityTokenIdentifier) {
        let context = BackendTaskContext::for_dispatch(&BackendTask::TokenTask(Box::new(
            TokenTask::QueryMyTokenBalances,
        )));
        let pair = IdentityTokenIdentifier {
            identity_id: Identifier::from([1; 32]),
            token_id: Identifier::from([2; 32]),
        };
        let mut state = claims(ctx);
        state.pending.push(ClaimInFlight {
            context: context.clone(),
            network,
            pair,
            distribution,
            scheduled_moment: at,
            token_name: "Reward".into(),
            progress: MessageBanner::set_global(
                ctx,
                format!("Claim in progress on {network}."),
                MessageType::Info,
            ),
        });
        store(ctx, state);
        (context, pair)
    }

    #[test]
    fn an_inactive_network_success_settles_only_its_own_claim_and_refresh() {
        let ctx = egui::Context::default();
        let (old, pair) = begin(
            &ctx,
            Network::Testnet,
            TokenDistributionType::PreProgrammed,
            Some(10),
        );
        let (active, _) = begin(
            &ctx,
            Network::Mainnet,
            TokenDistributionType::OncePerIdentity,
            None,
        );
        record_error(&ctx, &old, &TaskError::NetworkSwitchedBeforeResult);
        assert_eq!(
            claims(&ctx).pending.len(),
            2,
            "end-of-wait is not settlement"
        );
        let contexts = BTreeMap::new();
        let feedback = Feedback {
            ctx: &ctx,
            contexts: &contexts,
            delivery: ResultDelivery {
                sender_in_view: false,
                sender_gone: true,
            },
        };
        let updates = TokensFeedback.on_success(
            &feedback,
            &old,
            &BackendTaskSuccessResult::ClaimedTokens(FeeResult::estimated_only(1)),
        );
        assert!(matches!(
            updates.as_slice(),
            [CacheUpdate {
                network: Network::Testnet,
                domain: CacheDomain::Tokens
            }]
        ));
        let state = claims(&ctx);
        assert_eq!(state.pending.len(), 1);
        assert_eq!(state.pending[0].context, active);
        assert_eq!(
            state.receipts[&(Network::Testnet, pair)].scheduled_through,
            Some(10)
        );
        assert!(!state.receipts.contains_key(&(Network::Mainnet, pair)));
        assert_eq!(state.refresh, BTreeSet::from([Network::Testnet]));
        assert!(
            record_success(
                &ctx,
                &old,
                &BackendTaskSuccessResult::ClaimedTokens(FeeResult::estimated_only(1))
            )
            .is_none(),
            "duplicate delivery cannot consume another installment"
        );
    }

    #[test]
    fn an_unproven_once_refusal_does_not_consume_the_claim() {
        let ctx = egui::Context::default();
        let (request, pair) = begin(
            &ctx,
            Network::Testnet,
            TokenDistributionType::OncePerIdentity,
            None,
        );
        record_error(
            &ctx,
            &request,
            &TaskError::TokenOncePerIdentityAlreadyClaimed {
                token_id: pair.token_id,
                identity_id: pair.identity_id,
                claimed_at_ms: 10,
                source_error: Box::new(dash_sdk::Error::Generic("unproven test refusal".into())),
            },
        );
        assert!(claims(&ctx).pending.is_empty());
        assert!(
            claims(&ctx).receipts.is_empty(),
            "a newly opened form may explicitly retry"
        );
        assert!(claims(&ctx).refresh.is_empty());
    }

    #[test]
    fn a_claim_with_failed_receipt_storage_is_consumed_and_refreshed() {
        let ctx = egui::Context::default();
        let (request, pair) = begin(
            &ctx,
            Network::Testnet,
            TokenDistributionType::OncePerIdentity,
            None,
        );
        record_error(
            &ctx,
            &request,
            &TaskError::TokensClaimedButClaimNotRecorded {
                source: Box::new(TaskError::DataContractNotFound),
            },
        );
        let state = claims(&ctx);
        assert!(state.pending.is_empty());
        assert!(state.receipts[&(Network::Testnet, pair)].once);
        assert!(state.refresh.contains(&Network::Testnet));
    }

    #[test]
    fn abandoned_and_inactive_network_estimates_are_quiet_but_paid_errors_are_not() {
        for (abandoned, sender_gone, expected_quiet) in [
            (true, false, true),
            (false, true, true),
            (false, false, false),
        ] {
            let ctx = egui::Context::default();
            let request = BackendTaskContext::for_dispatch(&BackendTask::TokenTask(Box::new(
                TokenTask::EstimatePerpetualTokenRewardsWithExplanation {
                    identity_id: Identifier::from([1; 32]),
                    token_id: Identifier::from([2; 32]),
                },
            )));
            track_estimate(&ctx, request.clone());
            if abandoned {
                abandon_estimate(&ctx, &request);
            }
            let contexts = BTreeMap::new();
            let feedback = Feedback {
                ctx: &ctx,
                contexts: &contexts,
                delivery: ResultDelivery {
                    sender_in_view: !sender_gone,
                    sender_gone,
                },
            };
            assert_eq!(
                TokensFeedback.on_error(&feedback, &request, &TaskError::NoIdentitiesFound),
                expected_quiet
            );
            assert!(claims(&ctx).estimates.is_empty());
            let (paid, _) = begin(
                &ctx,
                Network::Testnet,
                TokenDistributionType::OncePerIdentity,
                None,
            );
            assert!(!TokensFeedback.on_error(&feedback, &paid, &TaskError::MasterKeyNotFound));
        }
    }
}
