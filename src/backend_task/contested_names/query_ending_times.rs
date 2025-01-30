use crate::context::AppContext;
use crate::model::proof_log_item::{ProofLogItem, RequestType};
use chrono::{DateTime, Duration, Utc};
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::dpp::voting::vote_polls::VotePoll;
use dash_sdk::drive::query::VotePollsByEndDateDriveQuery;
use dash_sdk::platform::FetchMany;
use std::collections::BTreeMap;
use std::sync::Arc;

use crate::app::TaskResult;
use dash_sdk::Sdk;
use tokio::sync::mpsc;

impl AppContext {
    pub(super) async fn query_dpns_ending_times(
        self: &Arc<Self>,
        sdk: Sdk,
        _sender: mpsc::Sender<TaskResult>,
    ) -> Result<(), String> {
        let now: DateTime<Utc> = Utc::now();
        let start_time_dt = now - Duration::weeks(2);
        let end_time_dt = now + Duration::weeks(2);
        let mut start_time = Some((start_time_dt.timestamp_millis() as u64, true));
        let end_time = Some((end_time_dt.timestamp_millis() as u64, true));

        let mut contests_end_times = BTreeMap::new();

        const MAX_RETRIES: usize = 3;
        let mut retries = 0;

        loop {
            let end_time_query = VotePollsByEndDateDriveQuery {
                start_time,
                end_time,
                limit: Some(100),
                offset: None,
                order_ascending: true,
            };

            let new_end_times = match VotePoll::fetch_many(&sdk, end_time_query.clone()).await {
                Ok(vote_polls) => {
                    let mut end_times = BTreeMap::new();
                    for (timestamp, vote_poll_list) in vote_polls {
                        let contests = vote_poll_list.into_iter().filter_map(|vote_poll| {
                            let VotePoll::ContestedDocumentResourceVotePoll(
                                ContestedDocumentResourceVotePoll {
                                    contract_id: _,
                                    document_type_name: _,
                                    index_name: _,
                                    index_values,
                                },
                            ) = vote_poll;

                            index_values
                                .get(1)
                                .and_then(|a| a.to_str().ok().map(|a| (a.to_string(), timestamp)))
                        });
                        end_times.extend(contests);
                    }
                    end_times
                }
                Err(e) => {
                    tracing::error!("Error fetching vote polls: {}", e);
                    if let dash_sdk::Error::Proof(dash_sdk::ProofVerifierError::GroveDBError {
                        proof_bytes,
                        height,
                        time_ms,
                        error,
                    }) = &e
                    {
                        // Encode the query using bincode
                        let encoded_query = match bincode::encode_to_vec(
                            &end_time_query,
                            bincode::config::standard(),
                        )
                        .map_err(|encode_err| {
                            tracing::error!("Error encoding query: {}", encode_err);
                            format!("Error encoding query: {}", encode_err)
                        }) {
                            Ok(encoded_query) => encoded_query,
                            Err(e) => return Err(e),
                        };

                        // Encode the path_query using bincode
                        let verification_path_query_bytes =
                            match bincode::encode_to_vec(&path_query, bincode::config::standard())
                                .map_err(|encode_err| {
                                    tracing::error!("Error encoding path_query: {}", encode_err);
                                    format!("Error encoding path_query: {}", encode_err)
                                }) {
                                Ok(encoded_path_query) => encoded_path_query,
                                Err(e) => return Err(e),
                            };

                        if let Err(e) = self
                            .db
                            .insert_proof_log_item(ProofLogItem {
                                request_type: RequestType::GetVotePollsByEndDate,
                                request_bytes: encoded_query,
                                verification_path_query_bytes,
                                height: *height,
                                time_ms: *time_ms,
                                proof_bytes: proof_bytes.clone(),
                                error: Some(error.clone()),
                            })
                            .map_err(|e| e.to_string())
                        {
                            return Err(e);
                        }
                    }
                    if e.to_string().contains("try another server")
                        || e.to_string().contains(
                            "contract not found when querying from value with contract info",
                        )
                    {
                        retries += 1;
                        if retries > MAX_RETRIES {
                            tracing::error!("Max retries reached for query: {}", e);
                            return Err(format!("Error fetching vote polls after retries: {}", e));
                        } else {
                            // Retry
                            continue;
                        }
                    } else {
                        return Err(format!("Error fetching vote polls: {}", e));
                    }
                }
            };

            contests_end_times.extend(new_end_times.clone());

            if new_end_times.len() == 0 {
                break;
            }

            let last_found_ending_time = new_end_times.values().max();
            if let Some(last_found_ending_time) = last_found_ending_time {
                start_time = Some((*last_found_ending_time, false));
            } else {
                break;
            }
        }

        self.db
            .update_ending_time(contests_end_times, self)
            .map_err(|e| format!("Error updating ending time: {}", e))
    }
}
