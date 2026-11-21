//! User feedback and cache effects of completed vote operations.
use super::copy::{DPNS_ALL_VOTES_ALREADY_CAST, dpns_vote_feedback, scheduled_vote_clear_feedback};
use crate::backend_task::error::TaskError;
use crate::backend_task::{BackendTaskContext, BackendTaskSuccessResult, CacheDomain, CacheUpdate};
use crate::ui::task_feedback::{DomainFeedback, Feedback};
use crate::ui::{MessageType, components::MessageBanner};

/// The voting domain's reactions.
pub(crate) struct DpnsFeedback;

impl DomainFeedback for DpnsFeedback {
    fn on_success(
        &self,
        feedback: &Feedback<'_>,
        _context: &BackendTaskContext,
        result: &BackendTaskSuccessResult,
    ) -> Vec<CacheUpdate> {
        let ctx = feedback.ctx;
        match result {
            BackendTaskSuccessResult::DpnsVoteOperationUpdated {
                network,
                operation_id,
            } => {
                let operation_context = feedback.contexts.get(network);
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
                        MessageBanner::set_global(
                            ctx,
                            DPNS_ALL_VOTES_ALREADY_CAST,
                            MessageType::Info,
                        );
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

    fn on_error(
        &self,
        _feedback: &Feedback<'_>,
        _context: &BackendTaskContext,
        error: &TaskError,
    ) -> bool {
        is_quiet(error)
    }
}

/// Whether a scheduled-vote sweep failure is expected and goes unreported:
/// the sweep found no voting identity on this device.
pub(crate) fn is_quiet(error: &TaskError) -> bool {
    matches!(
        error,
        TaskError::ScheduledVoteSweepFailed { source, .. }
            if matches!(source.as_ref(), TaskError::NoVotingIdentity { .. })
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::dashcore::Network;

    #[test]
    fn only_missing_local_identity_sweep_errors_are_quiet() {
        let missing_identity = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::NoVotingIdentity {
                identity_id: "voter-id".to_string(),
            }),
        };
        assert!(is_quiet(&missing_identity));

        let unavailable_result = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::DpnsCurrentVoteUnavailable),
        };
        assert!(!is_quiet(&unavailable_result));

        let nested_sweep = TaskError::ScheduledVoteSweepFailed {
            network: Network::Regtest,
            source: Box::new(TaskError::ScheduledVoteSweepFailed {
                network: Network::Regtest,
                source: Box::new(TaskError::NoVotingIdentity {
                    identity_id: "voter-id".to_string(),
                }),
            }),
        };
        assert!(!is_quiet(&nested_sweep));
        assert!(!is_quiet(&TaskError::NoVotingIdentity {
            identity_id: "voter-id".to_string(),
        }));
    }
}
