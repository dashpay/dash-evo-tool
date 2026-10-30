//! User feedback and cache effects of completed vote operations.
use super::copy::{DPNS_ALL_VOTES_ALREADY_CAST, dpns_vote_feedback, scheduled_vote_clear_feedback};
use crate::backend_task::{BackendTaskSuccessResult, CacheDomain, CacheUpdate};
use crate::context::AppContext;
use crate::ui::{MessageType, components::MessageBanner};
use dash_sdk::dpp::dashcore::Network;
use std::{collections::BTreeMap, sync::Arc};

pub(crate) fn show(
    ctx: &egui::Context,
    contexts: &BTreeMap<Network, Arc<AppContext>>,
    result: &BackendTaskSuccessResult,
) -> Vec<CacheUpdate> {
    match result {
        BackendTaskSuccessResult::DpnsVoteOperationUpdated {
            network,
            operation_id,
        } => {
            let operation_context = contexts.get(network);
            match operation_context
                .as_ref()
                .map(|context| context.dpns_vote_operation(*operation_id))
            {
                Some(Ok(Some(operation))) => {
                    let diagnostics = operation_context
                        .as_ref()
                        .map(|context| context.dpns_vote_operation_diagnostics(*operation_id))
                        .unwrap_or_default();
                    let (message, message_type, keep_visible) = dpns_vote_feedback(&operation);
                    let handle = MessageBanner::set_global(ctx, message, message_type);
                    if !diagnostics.is_empty() {
                        handle.with_details(&diagnostics);
                    }
                    if keep_visible {
                        handle.disable_auto_dismiss();
                    }
                }
                // A submit whose targets were all no-ops never
                // persists an operation, so the skipped count is
                // not recoverable here: use the plural copy.
                Some(Ok(None)) => {
                    MessageBanner::set_global(ctx, DPNS_ALL_VOTES_ALREADY_CAST, MessageType::Info);
                }
                Some(Err(error)) => tracing::warn!(
                    ?error,
                    ?network,
                    operation_id = %operation_id,
                    "Could not load DPNS vote operation feedback"
                ),
                None => tracing::warn!(
                    ?network,
                    operation_id = %operation_id,
                    "Could not find the originating network for DPNS vote feedback"
                ),
            }
            vec![CacheUpdate {
                network: *network,
                domain: CacheDomain::Dpns,
            }]
        }
        BackendTaskSuccessResult::ScheduledVotesCleared(outcomes) => {
            let (message, message_type) = scheduled_vote_clear_feedback(outcomes);
            MessageBanner::set_global(ctx, message, message_type);
            outcomes
                .iter()
                .map(|outcome| CacheUpdate {
                    network: outcome.key.network,
                    domain: CacheDomain::Dpns,
                })
                .collect()
        }
        _ => Vec::new(),
    }
}
