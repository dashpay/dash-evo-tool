//! Identity-side username ops: availability checks and own-request status.

use std::collections::BTreeMap;

use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::document::DocumentV0Getters;
use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::dpp::voting::vote_info_storage::contested_document_vote_poll_winner_info::ContestedDocumentVotePollWinnerInfo;
use dash_sdk::drive::query::{SelectProjection, WhereClause, WhereOperator};
use dash_sdk::platform::{Document, DocumentQuery, FetchMany, Identifier};
use dash_sdk::query_types::Contenders;
use dash_sdk::{Error as SdkError, Sdk};

use super::BackendTaskSuccessResult;
use crate::backend_task::error::TaskError;
use crate::context::AppContext;
use crate::model::dpns::{
    ContestDurations, DpnsNameValidationResult, contest_durations, is_contested_label,
    normalize_dpns_label, validate_dpns_name,
};
use crate::model::dpns_usernames::{
    ContenderTally, ContestSnapshot, ContestWinner, UsernameAvailability, UsernameRequest,
    classify_availability, merge_requests, username_request_from_contest,
};
use crate::utils::time::now_ms;

fn availability_error(source: SdkError) -> TaskError {
    TaskError::UsernameAvailabilityCheckFailed {
        source: Box::new(source),
    }
}

impl AppContext {
    fn username_contest_durations(&self, sdk: &Sdk) -> ContestDurations {
        contest_durations(self.network, sdk.version())
    }

    /// Whether `label` can be requested now, combining the registered-name
    /// lookup with the proved contest state. Never trusts
    /// `Sdk::is_dpns_name_available`, which reports contested and locked names as free.
    pub(crate) async fn username_availability(
        &self,
        sdk: &Sdk,
        label: &str,
    ) -> Result<UsernameAvailability, TaskError> {
        let validation = validate_dpns_name(label);
        if validation != DpnsNameValidationResult::Valid {
            return Err(TaskError::InvalidDpnsName { validation });
        }
        let normalized = normalize_dpns_label(label);
        let awarded = self.dpns_domain_exists(sdk, &normalized).await?;
        let contest = if !awarded && is_contested_label(label) {
            let contenders = sdk
                .get_contested_dpns_vote_state(&normalized, None)
                .await
                .map_err(availability_error)?;
            Some(self.contest_snapshot(sdk, &contenders))
        } else {
            None
        };
        Ok(classify_availability(
            label,
            awarded,
            contest.as_ref(),
            now_ms(),
            self.username_contest_durations(sdk),
        ))
    }

    pub(super) async fn check_username_availability(
        &self,
        sdk: &Sdk,
        label: String,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let availability = self.username_availability(sdk, &label).await?;
        Ok(BackendTaskSuccessResult::UsernameAvailability {
            label,
            availability,
        })
    }

    async fn dpns_domain_exists(&self, sdk: &Sdk, normalized: &str) -> Result<bool, TaskError> {
        let query = DocumentQuery {
            sub_queries: Vec::new(),
            select: SelectProjection::documents(),
            data_contract: self.dpns_contract.clone(),
            document_type_name: "domain".to_string(),
            where_clauses: vec![
                WhereClause {
                    field: "normalizedParentDomainName".to_string(),
                    operator: WhereOperator::Equal,
                    value: Value::Text("dash".to_string()),
                },
                WhereClause {
                    field: "normalizedLabel".to_string(),
                    operator: WhereOperator::Equal,
                    value: Value::Text(normalized.to_owned()),
                },
            ],
            time_range_clauses: Vec::new(),
            group_by: Vec::new(),
            having: Vec::new(),
            order_by_clauses: vec![],
            limit: 1,
            offset: None,
            start: None,
        };
        let documents = Document::fetch_many(sdk, query)
            .await
            .map_err(availability_error)?;
        Ok(documents.values().any(Option::is_some))
    }

    /// Convert a proved vote state into the pure snapshot the model decides on.
    fn contest_snapshot(&self, sdk: &Sdk, contenders: &Contenders) -> ContestSnapshot {
        let documents = self
            .dpns_contract
            .document_type_cloned_for_name("domain")
            .ok()
            .and_then(|document_type| {
                contenders
                    .to_contenders(&document_type, sdk.version())
                    .inspect_err(|error| {
                        tracing::debug!(?error, "Username contest documents could not be decoded")
                    })
                    .ok()
            })
            .unwrap_or_default();
        let contenders_list = contenders
            .contenders
            .iter()
            .map(|(identity_id, contender)| {
                let document = documents
                    .get(identity_id)
                    .and_then(|contender| contender.document().as_ref());
                ContenderTally {
                    identity_id: *identity_id,
                    label: document
                        .and_then(|doc| doc.get("label"))
                        .and_then(|label| label.to_str().ok())
                        .map(str::to_owned),
                    votes: contender.vote_tally().unwrap_or(0),
                    created_at: document.and_then(|doc| doc.created_at()),
                }
            })
            .collect();
        let (winner, decided_at) = match &contenders.winner {
            Some((info, block)) => (
                Some(match info {
                    ContestedDocumentVotePollWinnerInfo::WonByIdentity(id) => {
                        ContestWinner::Identity(*id)
                    }
                    ContestedDocumentVotePollWinnerInfo::Locked => ContestWinner::Locked,
                    ContestedDocumentVotePollWinnerInfo::NoWinner => ContestWinner::NoWinner,
                }),
                Some(block.time_ms),
            ),
            None => (None, None),
        };
        ContestSnapshot {
            winner,
            decided_at,
            contenders: contenders_list,
            lock_votes: contenders.lock_vote_tally.unwrap_or(0),
            abstain_votes: contenders.abstain_vote_tally.unwrap_or(0),
        }
    }

    /// Refresh the username requests of every loaded identity.
    ///
    /// One scan of running contests serves all identities. Requests that left
    /// the running set are re-read individually to learn their outcome.
    pub(super) async fn refresh_my_username_requests(
        &self,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let identity_ids: Vec<Identifier> = self
            .load_local_user_identities()?
            .iter()
            .map(|identity| identity.identity.id())
            .collect();
        let running = sdk
            .get_contested_non_resolved_usernames(None)
            .await
            .map_err(|source| TaskError::UsernameRequestRefreshFailed {
                source: Box::new(source),
            })?;
        let durations = self.username_contest_durations(sdk);
        let now = now_ms();
        let snapshots: BTreeMap<&String, (ContestSnapshot, u64)> = running
            .iter()
            .map(|(name, info)| {
                (
                    name,
                    (self.contest_snapshot(sdk, &info.contenders), info.end_time),
                )
            })
            .collect();

        for identity_id in identity_ids {
            let previous = self.username_requests_for(&identity_id);
            let mut fresh: Vec<UsernameRequest> = snapshots
                .iter()
                .filter_map(|(name, (snapshot, end))| {
                    username_request_from_contest(
                        identity_id,
                        name,
                        snapshot,
                        Some(*end),
                        now,
                        durations,
                    )
                })
                .collect();
            for old in previous.iter().filter(|old| {
                old.phase.is_pending() && !snapshots.contains_key(&old.normalized_label)
            }) {
                match sdk
                    .get_contested_dpns_vote_state(&old.normalized_label, None)
                    .await
                {
                    Ok(contenders) => {
                        let snapshot = self.contest_snapshot(sdk, &contenders);
                        if snapshot.winner.is_some()
                            && let Some(outcome) = username_request_from_contest(
                                identity_id,
                                &old.normalized_label,
                                &snapshot,
                                old.end,
                                now,
                                durations,
                            )
                        {
                            fresh.push(outcome);
                        }
                    }
                    Err(error) => tracing::debug!(
                        name = %old.normalized_label,
                        ?error,
                        "Username request outcome could not be read; keeping the stored status"
                    ),
                }
            }
            let merged = merge_requests(&previous, fresh, now);
            if merged != previous {
                self.store_username_requests(&identity_id, merged)?;
            }
        }
        Ok(BackendTaskSuccessResult::MyUsernameRequestsRefreshed)
    }

    /// Record a just-submitted contested request so the hub shows it at once.
    pub(super) fn record_submitted_username_request(
        &self,
        sdk: &Sdk,
        identity_id: &Identifier,
        label: &str,
    ) -> Result<(), TaskError> {
        let now = now_ms();
        let submitted =
            UsernameRequest::submitted(label, now, self.username_contest_durations(sdk));
        let previous = self.username_requests_for(identity_id);
        let merged = merge_requests(&previous, vec![submitted], now);
        self.store_username_requests(identity_id, merged)
    }
}
