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
    ContenderTally, ContestSnapshot, ContestWinner, RequestPhase, UsernameAvailability,
    UsernameRequest, classify_availability, mark_wins_reflected, merge_requests,
    username_request_from_contest,
};
use crate::model::qualified_identity::QualifiedIdentity;
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
    ///
    /// `requester` is the identity asking; its own running request reports
    /// `AlreadyRequested`, because Platform rejects a second request.
    pub(crate) async fn username_availability(
        &self,
        sdk: &Sdk,
        requester: Identifier,
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
            requester,
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
        identity_id: Identifier,
        label: String,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let availability = self.username_availability(sdk, identity_id, &label).await?;
        Ok(BackendTaskSuccessResult::UsernameAvailability {
            label,
            availability,
        })
    }

    /// The lookup for a registered name with this normalized label.
    pub(super) fn dpns_domain_query(&self, normalized: &str) -> DocumentQuery {
        DocumentQuery {
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
        }
    }

    async fn dpns_domain_exists(&self, sdk: &Sdk, normalized: &str) -> Result<bool, TaskError> {
        let documents = Document::fetch_many(sdk, self.dpns_domain_query(normalized))
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
    /// Saved pending requests are checked and published before the wider scan,
    /// so a slow or failing discovery query cannot hold their outcomes back.
    /// Discovery also finds requests made on other devices.
    pub(super) async fn refresh_my_username_requests(
        &self,
        sdk: &Sdk,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        let now = now_ms();
        let identities = self.load_local_user_identities()?;
        if identities.is_empty() {
            return Ok(BackendTaskSuccessResult::MyUsernameRequestsRefreshed);
        }
        let durations = self.username_contest_durations(sdk);
        let mut checked = BTreeMap::new();
        let mut first_error = None;
        for identity in &identities {
            let identity_id = identity.identity.id();
            let previous = self.username_requests_for(&identity_id);
            let mut fresh = Vec::new();
            for old in previous.iter().filter(|old| old.phase.is_pending()) {
                if !checked.contains_key(&old.normalized_label) {
                    match sdk
                        .get_contested_dpns_vote_state(&old.normalized_label, None)
                        .await
                    {
                        Ok(contenders) => {
                            checked.insert(
                                old.normalized_label.clone(),
                                self.contest_snapshot(sdk, &contenders),
                            );
                        }
                        Err(error) => tracing::debug!(
                            name = %old.normalized_label, ?error,
                            "Username request outcome could not be read; keeping the stored status"
                        ),
                    }
                }
                if let Some(snapshot) = checked.get(&old.normalized_label) {
                    fresh.extend(username_request_from_contest(
                        identity_id,
                        &old.normalized_label,
                        snapshot,
                        old.end,
                        now,
                        durations,
                    ));
                }
            }
            if let Err(error) = self.apply_username_refresh(sdk, identity, fresh).await {
                first_error.get_or_insert(error);
            }
        }
        // TODO(usernames): the SDK scan pages only while a timestamp group holds 100 polls and drops
        // names whose vote state fails; requests made on another device can be missed past that cap.
        let running = match sdk.get_contested_non_resolved_usernames(None).await {
            Ok(running) => running,
            Err(source) => {
                return Err(first_error.unwrap_or_else(|| {
                    TaskError::UsernameRequestRefreshFailed {
                        source: Box::new(source),
                    }
                }));
            }
        };
        let snapshots: BTreeMap<&String, (ContestSnapshot, u64)> = running
            .iter()
            .map(|(name, info)| {
                (
                    name,
                    (self.contest_snapshot(sdk, &info.contenders), info.end_time),
                )
            })
            .collect();

        for identity in identities {
            let identity_id = identity.identity.id();
            let previous = self.username_requests_for(&identity_id);
            let fresh: Vec<UsernameRequest> = snapshots
                .iter()
                .filter(|(name, _)| {
                    !previous.iter().any(|request| {
                        request.normalized_label == name.as_str() && !request.phase.is_pending()
                    })
                })
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
            if let Err(error) = self.apply_username_refresh(sdk, &identity, fresh).await {
                first_error.get_or_insert(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(BackendTaskSuccessResult::MyUsernameRequestsRefreshed)
    }

    async fn apply_username_refresh(
        &self,
        sdk: &Sdk,
        identity: &QualifiedIdentity,
        fresh: Vec<UsernameRequest>,
    ) -> Result<(), TaskError> {
        let identity_id = identity.identity.id();
        {
            let lock = self.identity_record_lock(identity_id);
            let _guard = lock
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !self.is_identity_listed(&identity_id)? {
                return Ok(());
            }
            let Some(current_identity) = self.get_local_qualified_identity(&identity_id)? else {
                return Ok(());
            };
            self.update_username_requests(&identity_id, |current| {
                merge_requests(current, fresh, now_ms(), Some(&current_identity.dpns_names))
            })?;
        }
        self.egui_ctx().request_repaint();
        self.record_won_usernames(sdk, identity_id).await;
        Ok(())
    }

    /// Retry saved wins until the owned names are durably reflected in the identity.
    async fn record_won_usernames(&self, sdk: &Sdk, identity_id: Identifier) {
        let requests = self.username_requests_for(&identity_id);
        if !requests
            .iter()
            .any(|request| request.phase == RequestPhase::Won)
        {
            self.update_username_hydration_status(identity_id, &[]);
            return;
        }
        let identity = match self.get_local_qualified_identity(&identity_id) {
            Ok(Some(identity)) => identity,
            Ok(None) => {
                return;
            }
            Err(error) => {
                tracing::warn!(?error, %identity_id, "Won username identity could not be read; refresh will retry");
                self.update_username_hydration_status(identity_id, &[]);
                return;
            }
        };
        let missing = self.update_username_hydration_status(identity_id, &identity.dpns_names);
        if !missing {
            return;
        }
        match self.fetch_owned_dpns_names(sdk, identity_id).await {
            Ok(names) => {
                match self.edit_local_qualified_identity(&identity_id, |fresh| {
                    fresh.dpns_names = names;
                    Ok(())
                }) {
                    Ok(saved) => {
                        // Recorded at once: a name transferred away before the
                        // next refresh must not look like a win still to read back.
                        if let Err(error) = self.update_username_requests(&identity_id, |current| {
                            let mut requests = current.to_vec();
                            mark_wins_reflected(&mut requests, &saved.dpns_names);
                            requests
                        }) {
                            tracing::warn!(?error, %identity_id, "Won username was saved but could not be marked as read back; the next refresh will mark it");
                        }
                        self.update_username_hydration_status(identity_id, &saved.dpns_names);
                    }
                    Err(error) => {
                        tracing::warn!(?error, %identity_id, "Won username could not be saved; refresh will retry")
                    }
                }
            }
            Err(error) => {
                tracing::debug!(?error, %identity_id, "Won username could not be fetched; refresh will retry")
            }
        }
    }

    /// Record a just-submitted contested request so the hub shows it at once.
    ///
    /// `joined_until` is the join deadline when the request joined a running contest.
    pub(super) fn record_submitted_username_request(
        &self,
        sdk: &Sdk,
        identity_id: &Identifier,
        label: &str,
        joined_until: Option<u64>,
    ) -> Result<(), TaskError> {
        let now = now_ms();
        let submitted = UsernameRequest::submitted(
            label,
            now,
            self.username_contest_durations(sdk),
            joined_until,
        );
        self.update_username_requests(identity_id, |current| {
            let previous: Vec<_> = current
                .iter()
                .filter(|request| request.normalized_label != submitted.normalized_label)
                .cloned()
                .collect();
            merge_requests(&previous, vec![submitted], now, None)
        })
    }

    /// Show `name` as the main username of `identity_id` on this device.
    pub(super) fn save_main_username(
        &self,
        identity_id: Identifier,
        name: String,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.set_main_username(&identity_id, &name)?;
        Ok(BackendTaskSuccessResult::UsernamePreferencesSaved)
    }

    /// Remove a finished request from the identity's list.
    pub(super) fn dismiss_username_outcome(
        &self,
        identity_id: Identifier,
        normalized_label: String,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.dismiss_username_request(&identity_id, &normalized_label)?;
        Ok(BackendTaskSuccessResult::UsernamePreferencesSaved)
    }

    /// Record that outcome banners were shown, so each appears only once.
    pub(super) fn save_username_outcomes_seen(
        &self,
        identity_id: Identifier,
        requests: Vec<UsernameRequest>,
    ) -> Result<BackendTaskSuccessResult, TaskError> {
        self.note_username_seen_mark(&identity_id, None);
        let result = self.mark_username_outcomes_seen(&identity_id, &requests);
        self.note_username_seen_mark(&identity_id, Some(result.is_ok()));
        result?;
        Ok(BackendTaskSuccessResult::UsernamePreferencesSaved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::dpp::block::block_info::BlockInfo;
    use dash_sdk::dpp::voting::contender_structs::ContenderWithSerializedDocument;
    use dash_sdk::drive::query::vote_poll_vote_state_query::{
        ContestedDocumentVotePollDriveQuery, ContestedDocumentVotePollDriveQueryResultType,
    };

    fn bare_identity(
        id: Identifier,
        network: dash_sdk::dpp::dashcore::Network,
    ) -> QualifiedIdentity {
        QualifiedIdentity {
            identity: dash_sdk::dpp::identity::Identity::create_basic_identity(
                id,
                dash_sdk::dpp::version::PlatformVersion::latest(),
            )
            .unwrap(),
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: crate::model::qualified_identity::IdentityType::User,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: crate::model::qualified_identity::IdentityStatus::Active,
            network,
        }
    }

    async fn sdk_with_owned_name(ctx: &AppContext, id: Identifier) -> Sdk {
        let query = DocumentQuery {
            sub_queries: vec![],
            select: SelectProjection::documents(),
            data_contract: ctx.dpns_contract.clone(),
            document_type_name: "domain".into(),
            where_clauses: vec![WhereClause {
                field: "records.identity".into(),
                operator: WhereOperator::Equal,
                value: Value::Identifier(id.into()),
            }],
            time_range_clauses: vec![],
            group_by: vec![],
            having: vec![],
            order_by_clauses: vec![],
            limit: 100,
            offset: None,
            start: None,
        };
        let document = Document::V0(dash_sdk::dpp::document::DocumentV0 {
            id: Identifier::from([88; 32]),
            owner_id: id,
            creator_id: None,
            properties: BTreeMap::from([("label".into(), Value::Text("Alice".into()))]),
            revision: None,
            created_at: Some(100),
            updated_at: None,
            transferred_at: None,
            created_at_block_height: None,
            updated_at_block_height: None,
            transferred_at_block_height: None,
            created_at_core_block_height: None,
            updated_at_core_block_height: None,
            transferred_at_core_block_height: None,
            contract_version: None,
        });
        let documents: dash_sdk::query_types::Documents =
            [(document.id(), Some(document))].into_iter().collect();
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch_many::<Identifier, Document, _, dash_sdk::query_types::Documents>(
                query,
                Some(documents),
            )
            .await
            .unwrap();
        sdk
    }

    #[tokio::test]
    async fn stale_pending_refresh_preserves_saved_win_and_hydration_retry() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let id = Identifier::from([42; 32]);
        let identity = bare_identity(id, ctx.network);
        ctx.insert_local_qualified_identity_sidecar_only(&identity)
            .unwrap();
        let sdk = Sdk::new_mock();
        let pending =
            UsernameRequest::submitted("Alice", 100, ctx.username_contest_durations(&sdk), None);
        let mut won = pending.clone();
        won.phase = RequestPhase::Won;
        won.last_updated = 200;
        won.decided_at = Some(200);
        ctx.store_username_requests(&id, vec![won.clone()]).unwrap();
        ctx.apply_username_refresh(&sdk, &identity, vec![pending])
            .await
            .unwrap();
        assert_eq!(ctx.username_requests_for(&id), vec![won.clone()]);
        assert!(ctx.username_requests_need_refresh());
        ctx.refresh_pending_dpns_usernames().unwrap();
        assert_eq!(ctx.username_requests_for(&id), vec![won]);
        assert!(ctx.username_requests_need_refresh());

        ctx.record_submitted_username_request(&sdk, &id, "Alice", None)
            .unwrap();
        assert_eq!(
            ctx.username_requests_for(&id)[0].phase,
            RequestPhase::Joinable
        );
        assert!(ctx.username_requests_for(&id)[0].requested_at.unwrap() > 200);
    }

    #[tokio::test]
    async fn removed_identity_ignores_an_awaited_username_refresh() {
        for phase in [RequestPhase::Joinable, RequestPhase::Won] {
            let dir = tempfile::tempdir().unwrap();
            let ctx = crate::context::test_support::test_app_context(dir.path());
            ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
                std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
            ));
            let id = Identifier::from([42; 32]);
            let captured = bare_identity(id, ctx.network);
            ctx.insert_local_qualified_identity_sidecar_only(&captured)
                .unwrap();
            let sdk = Sdk::new_mock();
            let mut request = UsernameRequest::submitted(
                "Alice",
                now_ms(),
                ctx.username_contest_durations(&sdk),
                None,
            );
            request.phase = phase;
            ctx.store_username_requests(&id, vec![request.clone()])
                .unwrap();
            let cleanup = ctx
                .delete_local_qualified_identity_with_outcome(&id)
                .unwrap();
            assert!(cleanup.is_incomplete(), "the fixture has no wallet backend");
            assert!(!ctx.is_identity_listed(&id).unwrap());
            ctx.apply_username_refresh(&sdk, &captured, vec![request])
                .await
                .unwrap();
            assert!(ctx.username_requests_for(&id).is_empty());
            assert!(!ctx.username_requests_need_refresh());
            ctx.refresh_pending_dpns_usernames().unwrap();
            assert!(ctx.username_requests_for(&id).is_empty());
            assert!(!ctx.username_requests_need_refresh());
        }
    }

    #[tokio::test]
    async fn username_refresh_reports_identity_load_failure() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let store =
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(store.clone()));
        store.fail_next_gets_containing("det:identity_index:v1", 1);
        assert!(
            ctx.refresh_my_username_requests(&Sdk::new_mock())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn username_refresh_reports_storage_failure_and_keeps_successful_siblings() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let store =
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(store.clone()));
        for byte in [42, 43] {
            let id = Identifier::from([byte; 32]);
            ctx.insert_local_qualified_identity_sidecar_only(&bare_identity(id, ctx.network))
                .unwrap();
            ctx.store_username_requests(
                &id,
                vec![UsernameRequest::submitted(
                    "Alice",
                    1,
                    ctx.username_contest_durations(&Sdk::new_mock()),
                    None,
                )],
            )
            .unwrap();
        }
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch_many::<Identifier, ContenderWithSerializedDocument, _, Contenders>(
                ContestedDocumentVotePollDriveQuery {
                    vote_poll: crate::model::dpns_voting::dpns_vote_poll(
                        &ctx.dpns_contract,
                        &normalize_dpns_label("Alice"),
                    )
                    .unwrap(),
                    result_type:
                        ContestedDocumentVotePollDriveQueryResultType::DocumentsAndVoteTally,
                    allow_include_locked_and_abstaining_vote_tally: true,
                    start_at: None,
                    limit: None,
                    offset: None,
                },
                Some(Contenders {
                    winner: Some((
                        ContestedDocumentVotePollWinnerInfo::Locked,
                        BlockInfo {
                            time_ms: now_ms(),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        sdk.mock().expect_fetch_many::<u64, dash_sdk::dpp::voting::vote_polls::VotePoll, _, dash_sdk::query_types::VotePollsGroupedByTimestamp>(
            dash_sdk::drive::query::VotePollsByEndDateDriveQuery {
                start_time: None, end_time: None, limit: Some(100), offset: None, order_ascending: true,
            },
            Some(Default::default()),
        ).await.unwrap();
        store.fail_next_puts_containing("det:username_requests:", 1);
        assert!(matches!(
            ctx.refresh_my_username_requests(&sdk).await,
            Err(TaskError::UsernameStorage { .. })
        ));
        let phases =
            [42, 43].map(|byte| ctx.username_requests_for(&Identifier::from([byte; 32]))[0].phase);
        assert_eq!(
            phases
                .iter()
                .filter(|phase| **phase == RequestPhase::Locked)
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn won_username_retries_fetch_after_restart() {
        won_username_retries_after_restart(false).await;
    }

    #[tokio::test]
    async fn won_username_retries_identity_write_after_restart() {
        won_username_retries_after_restart(true).await;
    }

    async fn won_username_retries_after_restart(fail_write: bool) {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        let store =
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::FailingKv::default());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(store.clone()));
        let id = Identifier::from([43; 32]);
        let identity = bare_identity(id, ctx.network);
        ctx.insert_local_qualified_identity_sidecar_only(&identity)
            .unwrap();
        let mut won = UsernameRequest::submitted(
            "Alice",
            now_ms(),
            ctx.username_contest_durations(&Sdk::new_mock()),
            None,
        );
        won.phase = RequestPhase::Won;
        won.decided_at = Some(now_ms());
        let sdk = if fail_write {
            sdk_with_owned_name(&ctx, id).await
        } else {
            Sdk::new_mock()
        };
        if fail_write {
            store.fail_next_puts_containing("det:identity:v1", 1);
        }
        ctx.apply_username_refresh(&sdk, &identity, vec![won.clone()])
            .await
            .unwrap();
        assert_eq!(ctx.username_requests_for(&id)[0].phase, RequestPhase::Won);
        assert!(
            ctx.username_requests_need_refresh(),
            "a saved win keeps periodic retries active"
        );
        assert!(
            ctx.get_local_qualified_identity(&id)
                .unwrap()
                .unwrap()
                .dpns_names
                .is_empty()
        );
        drop(ctx);
        let restarted_dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(restarted_dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(store));
        ctx.refresh_pending_dpns_usernames().unwrap();
        assert!(
            ctx.username_requests_need_refresh(),
            "startup restores the retry obligation"
        );
        let sdk = sdk_with_owned_name(&ctx, id).await;
        // Discovery is deliberately unavailable; saved winners must be repaired before it.
        let _ = ctx.refresh_my_username_requests(&sdk).await;
        let saved = ctx.get_local_qualified_identity(&id).unwrap().unwrap();
        assert_eq!(ctx.main_username(&saved).as_deref(), Some("Alice"));
        assert_eq!(saved.dpns_names.len(), 1);
        assert!(
            !ctx.username_requests_need_refresh(),
            "successful persistence clears the retry obligation"
        );
    }

    /// A won name is marked as read back the moment the identity stores it, so
    /// its later disappearance (the name left the identity) is not taken for a
    /// win still waiting to be read back, and is not fetched again.
    #[tokio::test]
    async fn saved_win_is_marked_once_the_identity_lists_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let id = Identifier::from([44; 32]);
        let identity = bare_identity(id, ctx.network);
        ctx.insert_local_qualified_identity_sidecar_only(&identity)
            .unwrap();
        let mut won = UsernameRequest::submitted(
            "Alice",
            now_ms(),
            ctx.username_contest_durations(&Sdk::new_mock()),
            None,
        );
        won.phase = RequestPhase::Won;
        won.decided_at = Some(now_ms());
        let sdk = sdk_with_owned_name(&ctx, id).await;

        ctx.apply_username_refresh(&sdk, &identity, vec![won])
            .await
            .unwrap();

        assert!(ctx.username_requests_for(&id)[0].reflected_in_owned_names);
        assert!(!ctx.username_requests_need_refresh());

        // The name has since left the identity; no owned-names read is mocked,
        // so a renewed attempt to read the win back would keep the retry alive.
        let without_name = ctx
            .edit_local_qualified_identity(&id, |identity| {
                identity.dpns_names.clear();
                Ok(())
            })
            .unwrap();
        ctx.apply_username_refresh(&Sdk::new_mock(), &without_name, vec![])
            .await
            .unwrap();

        assert!(ctx.username_requests_for(&id)[0].reflected_in_owned_names);
        assert!(!ctx.username_requests_need_refresh());
    }

    #[tokio::test]
    async fn known_username_outcome_refresh_survives_discovery_failure() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = crate::context::test_support::test_app_context(dir.path());
        ctx.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(
            std::sync::Arc::new(crate::wallet_backend::kv_test_support::InMemoryKv::default()),
        ));
        let id = Identifier::from([42; 32]);
        let identity = bare_identity(id, ctx.network);
        ctx.insert_local_qualified_identity_sidecar_only(&identity)
            .unwrap();
        ctx.store_username_requests(
            &id,
            vec![UsernameRequest::submitted(
                "d1ssh",
                1,
                ContestDurations {
                    total: std::time::Duration::from_secs(10),
                    join: std::time::Duration::from_secs(5),
                },
                None,
            )],
        )
        .unwrap();
        let mut sdk = Sdk::new_mock();
        sdk.mock()
            .expect_fetch_many::<Identifier, ContenderWithSerializedDocument, _, Contenders>(
                ContestedDocumentVotePollDriveQuery {
                    vote_poll: crate::model::dpns_voting::dpns_vote_poll(
                        &ctx.dpns_contract,
                        "d1ssh",
                    )
                    .unwrap(),
                    result_type:
                        ContestedDocumentVotePollDriveQueryResultType::DocumentsAndVoteTally,
                    allow_include_locked_and_abstaining_vote_tally: true,
                    start_at: None,
                    limit: None,
                    offset: None,
                },
                Some(Contenders {
                    winner: Some((
                        ContestedDocumentVotePollWinnerInfo::Locked,
                        BlockInfo {
                            time_ms: now_ms(),
                            ..Default::default()
                        },
                    )),
                    ..Default::default()
                }),
            )
            .await
            .unwrap();
        // No discovery expectation: scanning unrelated contests fails.
        assert!(matches!(
            ctx.refresh_my_username_requests(&sdk).await,
            Err(TaskError::UsernameRequestRefreshFailed { .. })
        ));
        ctx.refresh_pending_dpns_usernames().unwrap();
        assert_eq!(
            ctx.username_requests_for(&id)[0].phase,
            RequestPhase::Locked
        );
    }
}
