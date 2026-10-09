use crate::app::TaskResult;
use crate::backend_task::error::{DapiAddressAvailability, TaskError};
use crate::backend_task::{BackendTaskSuccessResult, contextualize_dapi_task_result};
use crate::context::AppContext;
use crate::model::request_type::RequestType;
use dash_sdk::Sdk;
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::data_contract::document_type::accessors::DocumentTypeV0Getters;
use dash_sdk::dpp::platform_value::Value;
use dash_sdk::drive::query::vote_polls_by_document_type_query::VotePollsByDocumentTypeQuery;
use dash_sdk::platform::FetchMany;
use dash_sdk::query_types::ContestedResource;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

/// Extra attempts one contest page query gets after a retryable failure.
const MAX_PAGE_RETRIES: usize = 3;
/// Pause before the first retry of a page query; each later retry waits longer.
const PAGE_RETRY_DELAY: Duration = Duration::from_millis(500);
/// Contender queries a pass starts at once, before it begins pacing them.
/// Open contests are queued first, so this is what brings them up at once.
const UNPACED_CONTENDER_QUERIES: usize = 100;
/// Shortest gap between the starts of two later contender queries of a pass.
///
/// The SDK sends requests to five endpoints at a time and each allows 150 a
/// minute, so the whole app has about 750 a minute. The first load of a long
/// contest history takes at most 100 + 480 of them in its first minute.
const CONTENDER_QUERY_SPACING: Duration = Duration::from_millis(125);

/// Whether asking again, likely on another node, can answer a failed page query.
fn page_query_is_retryable(error: &dash_sdk::Error) -> bool {
    // TODO(#875): replace substring match once the SDK exposes a
    // structural "contract not found" variant.
    matches!(error, dash_sdk::Error::StaleNode(_))
        || error
            .to_string()
            .contains("contract not found when querying from value with contract info")
}

/// Run one contest page query, retrying a retryable failure at most
/// [`MAX_PAGE_RETRIES`] times with a growing pause in between.
async fn fetch_page_with_retries<T, Fut>(
    retry_delay: Duration,
    mut fetch: impl FnMut() -> Fut,
) -> Result<T, TaskError>
where
    Fut: Future<Output = Result<T, dash_sdk::Error>>,
{
    let mut retries: u32 = 0;
    loop {
        let error = match fetch().await {
            Ok(page) => return Ok(page),
            Err(error) => error,
        };
        tracing::error!("Error fetching contested resources: {}", error);
        super::log_contested_proof_error(&error, RequestType::GetContestedResources);
        if !page_query_is_retryable(&error) {
            return Err(TaskError::from(error));
        }
        if retries as usize >= MAX_PAGE_RETRIES {
            tracing::error!("Max retries reached for query: {}", error);
            return Err(TaskError::from(error));
        }
        retries += 1;
        tokio::time::sleep(retry_delay.saturating_mul(retries)).await;
    }
}

/// Wait for every contest query of a pass and tell whether all of them
/// succeeded. One that failed, or did not run to its end, left its contest
/// data stale.
async fn every_contest_query_succeeded(queries: Vec<JoinHandle<bool>>) -> bool {
    let mut succeeded = true;
    for query in queries {
        match query.await {
            Ok(refreshed) => succeeded &= refreshed,
            Err(e) => {
                tracing::error!("Task failed: {:?}", e);
                succeeded = false;
            }
        }
    }
    succeeded
}

/// Start one query per name, in order: the first `unpaced` at once, each
/// later one no sooner than `spacing` after the one before it.
async fn start_in_order(
    names: Vec<String>,
    unpaced: usize,
    spacing: Duration,
    mut start: impl FnMut(String) -> JoinHandle<bool>,
) -> Vec<JoinHandle<bool>> {
    let mut pace = tokio::time::interval(spacing);
    pace.set_missed_tick_behavior(MissedTickBehavior::Delay);
    let mut queries = Vec::with_capacity(names.len());
    for (index, name) in names.into_iter().enumerate() {
        if index >= unpaced {
            pace.tick().await;
        }
        queries.push(start(name));
    }
    queries
}

impl AppContext {
    /// Prove every loaded node's current votes and refresh which nodes are in
    /// the masternode list. Tells whether every node's votes were proved.
    async fn prove_current_votes(&self, sdk: &Sdk) -> bool {
        let proved = match self.refresh_dpns_vote_states(sdk).await {
            Ok(results) => super::refresh_vote_states::every_voter_refreshed(&results),
            Err(error) => {
                tracing::warn!(?error, "Could not refresh DPNS current votes with contests");
                false
            }
        };
        self.refresh_masternode_list_membership().await;
        proved
    }

    /// Refresh the contest cache, contenders, end times and every loaded node's
    /// proved votes as one snapshot.
    ///
    /// `quiet` (the background timer) logs per-contest failures instead of
    /// sending them to the UI, so an offline app does not raise a banner every
    /// few minutes.
    pub(super) async fn query_dpns_contested_resources(
        self: &Arc<Self>,
        sdk: &Sdk,
        sender: crate::utils::egui_mpsc::SenderAsync<TaskResult>,
        quiet: bool,
    ) -> Result<(), TaskError> {
        // Held to the end of the pass and taken before any other lock, so a
        // pass that waits here holds nothing a running pass needs.
        let _pass = self.dpns_contest_refresh_pass.lock().await;
        let data_contract = self.dpns_contract.as_ref();
        let document_type = data_contract
            .document_type_for_name("domain")
            .map_err(|_| TaskError::ContractSchemaMismatch {
                detail: "DPNS contract missing 'domain' document type",
            })?;
        let Some(contested_index) = document_type.find_contested_index() else {
            return Err(TaskError::ContractSchemaMismatch {
                detail: "No contested index found on DPNS domain document type",
            });
        };
        let mut start_at_value = None;
        let mut names_to_be_updated = Vec::new();
        loop {
            let query = VotePollsByDocumentTypeQuery {
                contract_id: data_contract.id(),
                document_type_name: document_type.name().to_string(),
                index_name: contested_index.name.clone(),
                start_at_value: start_at_value.clone(),
                start_index_values: vec!["dash".into()], // hardcoded for dpns
                end_index_values: vec![],
                limit: Some(100),
                order_ascending: true,
            };

            let contested_resources = fetch_page_with_retries(PAGE_RETRY_DELAY, || {
                ContestedResource::fetch_many(sdk, query.clone())
            })
            .await?;
            let contested_resources_len = contested_resources.0.len();

            if contested_resources_len == 0 {
                break;
            }

            let contested_resources_as_strings: Vec<String> = contested_resources
                .0
                .into_iter()
                .filter_map(|contested_resource| match contested_resource.0.as_str() {
                    Some(s) => Some(s.to_string()),
                    None => {
                        tracing::warn!(
                            "Contested resource value is not a string: {:?}",
                            contested_resource.0
                        );
                        None
                    }
                })
                .collect();

            let Some(last_found_name) = contested_resources_as_strings.last().cloned() else {
                break;
            };

            let new_names_to_be_updated =
                self.insert_name_contests_as_normalized_names(contested_resources_as_strings)?;

            names_to_be_updated.extend(new_names_to_be_updated);

            sender
                .send(TaskResult::DpnsContestsChanged)
                .await
                .map_err(|_| TaskError::InternalSendError)?;

            if contested_resources_len < 100 {
                break;
            }
            start_at_value = Some((Value::Text(last_found_name), false))
        }

        let semaphore = Arc::new(Semaphore::new(24));

        let handle = {
            let semaphore = semaphore.clone();
            let sdk = sdk.clone();
            let sender = sender.clone();
            let self_ref = self.clone();

            tokio::spawn(async move {
                let _permit: OwnedSemaphorePermit = match semaphore.acquire_owned().await {
                    Ok(permit) => permit,
                    Err(e) => {
                        tracing::error!("Semaphore closed while querying dpns end times: {}", e);
                        return false;
                    }
                };

                match self_ref
                    .query_dpns_ending_times(sdk.clone(), sender.clone())
                    .await
                {
                    Ok(_) => {
                        if let Err(e) = sender.send(TaskResult::DpnsContestsChanged).await {
                            tracing::warn!(
                                "Failed to send refresh after dpns end times query: {}",
                                e
                            );
                        }
                        true
                    }
                    Err(e) if quiet => {
                        tracing::debug!(error = ?e, "Background refresh could not query contest end times");
                        false
                    }
                    Err(e) => {
                        tracing::error!("Error querying dpns end times: {}", e);
                        let result = contextualize_dapi_task_result(
                            TaskResult::unattributed_error(e),
                            DapiAddressAvailability::from_sdk(&sdk),
                        );
                        if let Err(send_err) = sender.send(result).await {
                            tracing::warn!(
                                "Failed to send error for dpns end times query: {}",
                                send_err
                            );
                        }
                        false
                    }
                }
            })
        };

        // The end times tell which contests are still open. Those are read
        // first, so a long unread history does not hold up what can be voted on.
        let end_times_refreshed = every_contest_query_succeeded(vec![handle]).await;
        let queue = self.contender_queue(names_to_be_updated)?;
        let reads_unread_history = !queue.unread.is_empty();

        let start_contender_query = |name: String| {
            let semaphore = semaphore.clone();
            let sdk = sdk.clone();
            let sender = sender.clone();
            let self_ref = self.clone();

            tokio::spawn(async move {
                let _permit: OwnedSemaphorePermit = match semaphore.acquire_owned().await {
                    Ok(permit) => permit,
                    Err(e) => {
                        tracing::error!(
                            "Semaphore closed while querying vote contenders for {}: {}",
                            name,
                            e
                        );
                        return false;
                    }
                };

                match self_ref
                    .query_dpns_vote_contenders(&name, &sdk, sender.clone())
                    .await
                {
                    Ok(_) => {
                        if let Err(e) = sender.send(TaskResult::DpnsContestsChanged).await {
                            tracing::warn!(
                                "Failed to send refresh after vote contenders query for {}: {}",
                                name,
                                e
                            );
                        }
                        true
                    }
                    Err(e) if quiet => {
                        tracing::debug!(error = ?e, %name, "Background refresh could not query contenders");
                        false
                    }
                    Err(e) => {
                        tracing::error!("Error querying dpns vote contenders for {}: {}", name, e);
                        let result = contextualize_dapi_task_result(
                            TaskResult::unattributed_error(e),
                            DapiAddressAvailability::from_sdk(&sdk),
                        );
                        if let Err(send_err) = sender.send(result).await {
                            tracing::warn!(
                                "Failed to send error for vote contenders query for {}: {}",
                                name,
                                send_err
                            );
                        }
                        false
                    }
                }
            })
        };
        let unpaced_after_open = UNPACED_CONTENDER_QUERIES.saturating_sub(queue.open.len());
        let mut handles = start_in_order(
            queue.open,
            UNPACED_CONTENDER_QUERIES,
            CONTENDER_QUERY_SPACING,
            &start_contender_query,
        )
        .await;
        let mut contests_refreshed = end_times_refreshed;
        if reads_unread_history {
            // Unread names are not listed as open, so the open contests on
            // show are the real ones: prove the nodes' votes on them now
            // rather than after the whole history is read.
            contests_refreshed &= every_contest_query_succeeded(std::mem::take(&mut handles)).await;
            self.prove_current_votes(sdk).await;
            self.recompute_dpns_vote_attention();
            sender
                .send(TaskResult::DpnsContestsChanged)
                .await
                .map_err(|_| TaskError::InternalSendError)?;
        }
        handles.extend(
            start_in_order(
                queue.rest,
                unpaced_after_open,
                CONTENDER_QUERY_SPACING,
                &start_contender_query,
            )
            .await,
        );
        contests_refreshed &= every_contest_query_succeeded(handles).await;

        // Publish contests and every loaded node's proved current votes as one
        // completed refresh snapshot. Per-node failures are stored explicitly
        // as unavailable instead of being mistaken for "Not voted".
        let vote_states_refreshed = self.prove_current_votes(sdk).await;
        self.forget_closed_dpns_vote_counts();
        self.recompute_dpns_vote_attention();
        self.refresh_pending_dpns_usernames()?;
        // A pass that left a contest stale or a node unchecked is not a
        // completed refresh: the timer and the Votes-arrival refresh must be
        // free to try again soon.
        if contests_refreshed && vote_states_refreshed {
            self.mark_dpns_contests_refreshed();
        }

        sender
            .send(TaskResult::unattributed_success(
                BackendTaskSuccessResult::RefreshedDpnsContests,
            ))
            .await
            .map_err(|_| TaskError::InternalSendError)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dash_sdk::error::StaleNodeError;
    use dash_sdk::platform::Identifier;
    use dash_sdk::query_types::ContestedResources;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn stale_node() -> dash_sdk::Error {
        dash_sdk::Error::StaleNode(StaleNodeError::Height {
            expected_height: 10,
            received_height: 1,
            tolerance_blocks: 1,
        })
    }

    /// A node that keeps answering as stale must end the query, not loop on it.
    #[tokio::test]
    async fn a_persistently_stale_node_ends_the_page_query_after_bounded_retries() {
        let attempts = AtomicUsize::new(0);
        let result: Result<(), _> = fetch_page_with_retries(Duration::ZERO, || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(stale_node())
        })
        .await;

        assert!(matches!(result, Err(TaskError::DapiStaleNode { .. })));
        assert_eq!(attempts.load(Ordering::SeqCst), MAX_PAGE_RETRIES + 1);
    }

    #[tokio::test]
    async fn a_page_query_recovers_when_a_retry_succeeds() {
        let attempts = AtomicUsize::new(0);
        let result = fetch_page_with_retries(Duration::ZERO, || async {
            if attempts.fetch_add(1, Ordering::SeqCst) == 0 {
                Err(stale_node())
            } else {
                Ok(7)
            }
        })
        .await;

        assert!(matches!(result, Ok(7)));
        assert_eq!(attempts.load(Ordering::SeqCst), 2);
    }

    /// A pass is complete only when every contest query ran to its end and
    /// succeeded.
    #[tokio::test]
    async fn contest_queries_succeed_only_when_every_one_of_them_did() {
        let query = |refreshed: bool| tokio::spawn(async move { refreshed });
        assert!(every_contest_query_succeeded(vec![]).await);
        assert!(every_contest_query_succeeded(vec![query(true), query(true)]).await);
        assert!(!every_contest_query_succeeded(vec![query(false), query(true)]).await);

        let stopped = tokio::spawn(std::future::pending::<bool>());
        stopped.abort();
        assert!(!every_contest_query_succeeded(vec![stopped, query(true)]).await);
    }

    /// A context on an in-memory store and a mock SDK that lists `names` as
    /// the contested names. Every other query of a pass fails at once.
    async fn context_listing(names: &[String]) -> (tempfile::TempDir, Arc<AppContext>, Sdk) {
        let dir = tempfile::tempdir().expect("tempdir");
        let context = crate::context::test_support::test_app_context(dir.path());
        context.set_det_kv_override_for_test(crate::wallet_backend::DetKv::from_store(Arc::new(
            crate::wallet_backend::kv_test_support::InMemoryKv::default(),
        )));
        let document_type = context
            .dpns_contract
            .document_type_for_name("domain")
            .expect("domain document type");
        let page_from = |start_at_value| VotePollsByDocumentTypeQuery {
            contract_id: context.dpns_contract.id(),
            document_type_name: document_type.name().to_owned(),
            index_name: document_type
                .find_contested_index()
                .expect("contested index")
                .name
                .clone(),
            start_at_value,
            start_index_values: vec!["dash".into()],
            end_index_values: vec![],
            limit: Some(100),
            order_ascending: true,
        };
        // The whole list is one page; a full page is followed by an empty one.
        let mut pages = vec![(None, names.to_vec())];
        if let Some(last) = names.last().filter(|_| names.len() >= 100) {
            pages.push((Some((Value::Text(last.clone()), false)), Vec::new()));
        }
        let mut sdk = Sdk::new_mock();
        for (start_at_value, page) in pages {
            sdk.mock()
                .expect_fetch_many::<Identifier, ContestedResource, _, ContestedResources>(
                    page_from(start_at_value),
                    Some(ContestedResources(
                        page.into_iter()
                            .map(|name| ContestedResource(Value::Text(name)))
                            .collect(),
                    )),
                )
                .await
                .expect("contest list expectation");
        }
        (dir, context, sdk)
    }

    fn contest_names(count: usize) -> Vec<String> {
        (0..count)
            .map(|index| format!("contest-{index:03}"))
            .collect()
    }

    /// Run one background refresh pass to its end.
    async fn run_pass(context: &Arc<AppContext>, sdk: &Sdk) -> Result<(), TaskError> {
        let (tx, _rx) = tokio::sync::mpsc::channel(16);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, context.egui_ctx().clone());
        context
            .query_dpns_contested_resources(sdk, sender, true)
            .await
    }

    /// The first load of a contest history has one contender query per name;
    /// past the allowance they must be spread over time, not sent as one burst.
    #[tokio::test]
    async fn contender_queries_past_the_allowance_are_spaced_out() {
        const PACED: u32 = 6;
        let names = contest_names(UNPACED_CONTENDER_QUERIES + PACED as usize);
        let (_dir, context, sdk) = context_listing(&names).await;

        let started = std::time::Instant::now();
        run_pass(&context, &sdk).await.expect("refresh pass");

        // The first paced query starts at once; each later one waits its turn.
        assert!(
            started.elapsed() >= CONTENDER_QUERY_SPACING * (PACED - 1),
            "{PACED} paced contender queries were started within {:?}",
            started.elapsed()
        );
    }

    /// Queries within the allowance are not held back by the pace.
    #[tokio::test]
    async fn contender_queries_within_the_allowance_start_at_once() {
        let names = contest_names(20);
        let allowance = names.len();

        // The pace is long enough that waiting for it even once runs out the clock.
        let started = tokio::time::timeout(
            Duration::from_secs(5),
            start_in_order(names, allowance, Duration::from_secs(60), |_| {
                tokio::spawn(async { true })
            }),
        )
        .await;

        assert!(
            started.is_ok_and(|queries| queries.len() == allowance),
            "a query within the allowance waited for the pace"
        );
    }

    /// On a cold cache a few open contests sit among a long unread history.
    /// Every open one must be asked about before any of the rest.
    #[tokio::test]
    async fn open_contests_are_dispatched_before_unread_history() {
        const HOUR_MS: u64 = 3_600_000;
        let (_dir, context, _sdk) = context_listing(&[]).await;
        let names = contest_names(400);
        let open: Vec<String> = names.iter().skip(7).step_by(20).cloned().collect();
        let ended = names.iter().skip(3).step_by(20).cloned();
        let now = crate::utils::time::now_ms();
        let listed = context
            .insert_name_contests_as_normalized_names(names.clone())
            .expect("list the contests on a cold cache");
        context
            .update_contested_name_ending_times(
                open.iter()
                    .map(|name| (name.clone(), now + HOUR_MS))
                    .chain(ended.map(|name| (name, now - HOUR_MS))),
            )
            .expect("store end times");

        let queue = context
            .contender_queue(listed)
            .expect("order the contender queries");
        let queue = queue.open.into_iter().chain(queue.rest).collect();
        let dispatched = Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = Arc::clone(&dispatched);
        let queries = start_in_order(queue, usize::MAX, Duration::from_millis(1), move |name| {
            record.lock().expect("dispatch record").push(name);
            tokio::spawn(async { true })
        })
        .await;
        assert!(every_contest_query_succeeded(queries).await);

        let dispatched = dispatched.lock().expect("dispatch record");
        assert_eq!(dispatched.len(), names.len());
        assert_eq!(dispatched[..open.len()], open[..]);
    }

    /// Two passes at once would each query the contests the other is already
    /// reading, so a second pass waits for the running one.
    #[tokio::test]
    async fn a_second_refresh_pass_waits_for_the_running_one() {
        let (_dir, context, sdk) = context_listing(&[]).await;

        let running = context.dpns_contest_refresh_pass.lock().await;
        let mut second = std::pin::pin!(run_pass(&context, &sdk));
        assert!(
            tokio::time::timeout(Duration::from_millis(200), &mut second)
                .await
                .is_err(),
            "a pass must not run while another one is still running"
        );

        drop(running);
        second.await.expect("the waiting pass runs afterwards");
    }

    /// A pass runs for minutes while the user works elsewhere. It reports that
    /// contest data changed; a screen-wide refresh would reset that screen.
    #[tokio::test]
    async fn a_pass_reports_contest_changes_without_refreshing_every_screen() {
        let (_dir, context, sdk) = context_listing(&contest_names(2)).await;
        let (tx, mut rx) = tokio::sync::mpsc::channel(16);
        let sender = crate::utils::egui_mpsc::SenderAsync::new(tx, context.egui_ctx().clone());

        context
            .query_dpns_contested_resources(&sdk, sender, true)
            .await
            .expect("refresh pass");

        let sent: Vec<TaskResult> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert!(
            !sent
                .iter()
                .any(|result| matches!(result, TaskResult::Refresh)),
            "a pass must not ask whatever screen is visible to refresh: {sent:?}"
        );
        assert!(
            sent.iter()
                .any(|result| matches!(result, TaskResult::DpnsContestsChanged)),
            "a pass must still report that contest data changed: {sent:?}"
        );
    }

    /// Load a masternode into `context` and let the mock SDK prove that it
    /// has not voted on anything.
    async fn load_node_without_votes(context: &Arc<AppContext>, sdk: &mut Sdk) -> Identifier {
        use crate::model::qualified_identity::{IdentityStatus, IdentityType, QualifiedIdentity};
        use dash_sdk::dpp::identity::accessors::IdentityGettersV0;
        use dash_sdk::dpp::voting::votes::resource_vote::ResourceVote;
        use dash_sdk::drive::query::contested_resource_votes_given_by_identity_query::ContestedResourceVotesGivenByIdentityQuery;
        use dash_sdk::query_types::ResourceVotesByIdentity;

        let (tx, _rx) = tokio::sync::mpsc::channel::<TaskResult>(8);
        context
            .ensure_wallet_backend(crate::utils::egui_mpsc::SenderAsync::new(
                tx,
                context.egui_ctx().clone(),
            ))
            .await
            .expect("wire the wallet backend offline");
        let identity = dash_sdk::dpp::identity::Identity::create_basic_identity(
            Identifier::from([1; 32]),
            dash_sdk::dpp::version::PlatformVersion::latest(),
        )
        .expect("basic identity");
        let node = QualifiedIdentity {
            identity,
            associated_voter_identity: None,
            associated_operator_identity: None,
            associated_owner_key_id: None,
            identity_type: IdentityType::Masternode,
            alias: None,
            private_keys: Default::default(),
            dpns_names: vec![],
            associated_wallets: Default::default(),
            secret_access: None,
            wallet_index: None,
            top_ups: Default::default(),
            status: IdentityStatus::Active,
            network: context.network(),
        };
        context
            .insert_local_qualified_identity(&node, &None)
            .expect("load the node");
        let node_id = node.identity.id();
        sdk.mock()
            .expect_fetch_many::<Identifier, ResourceVote, _, ResourceVotesByIdentity>(
                ContestedResourceVotesGivenByIdentityQuery {
                    identity_id: node_id,
                    offset: None,
                    limit: Some(100),
                    start_at: None,
                    order_ascending: true,
                },
                Some(ResourceVotesByIdentity::default()),
            )
            .await
            .expect("vote query expectation");
        node_id
    }

    /// On a first load the node's votes are proved as soon as the open
    /// contest is read. The unread history is still being back-filled then,
    /// and none of it may show as a contest the node has yet to vote on —
    /// nor after the pass is stopped midway, with the proof already stored.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn votes_are_proved_before_the_unread_history_is_read() {
        // Enough unread names for a paced tail of about two seconds; the open
        // contest is listed last.
        const PACED: usize = 16;
        let mut names = contest_names(UNPACED_CONTENDER_QUERIES + PACED - 1);
        names.push("open".to_owned());
        let (_dir, context, mut sdk) = context_listing(&names).await;
        let node_id = load_node_without_votes(&context, &mut sdk).await;
        context.seed_dpns_contest_for_test(
            "open",
            Some(crate::utils::time::now_ms() + 600_000),
            false,
        );

        let pass = tokio::spawn({
            let context = Arc::clone(&context);
            async move { run_pass(&context, &sdk).await }
        });
        // Contests the node is shown as able to decide on, sampled for a
        // second: well inside the back-fill.
        let mut decidable = Vec::new();
        for _ in 0..40 {
            tokio::time::sleep(Duration::from_millis(25)).await;
            let rows = context.dpns_node_votes(node_id).expect("node votes");
            decidable.push(
                rows.into_iter()
                    .filter(|row| row.state_known)
                    .map(|row| row.contested_name)
                    .collect::<Vec<_>>(),
            );
        }

        assert!(!pass.is_finished(), "the back-fill must still be running");
        assert!(
            decidable.iter().all(|names| names.len() <= 1),
            "unread history was shown as waiting for a vote: {decidable:?}"
        );
        assert_eq!(decidable.last().expect("samples"), &["open"]);

        pass.abort();
        assert!(pass.await.is_err_and(|error| error.is_cancelled()));
        let still_decidable: Vec<String> = context
            .dpns_node_votes(node_id)
            .expect("node votes")
            .into_iter()
            .map(|row| row.contested_name)
            .collect();
        assert_eq!(still_decidable, ["open"]);
    }

    #[tokio::test]
    async fn a_failure_another_node_cannot_fix_is_not_retried() {
        let attempts = AtomicUsize::new(0);
        let result: Result<(), _> = fetch_page_with_retries(Duration::ZERO, || async {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(dash_sdk::Error::Generic("unreachable".to_owned()))
        })
        .await;

        assert!(result.is_err());
        assert_eq!(attempts.load(Ordering::SeqCst), 1);
    }
}
