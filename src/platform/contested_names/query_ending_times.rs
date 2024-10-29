use crate::context::AppContext;
use chrono::{DateTime, Duration, Utc};
use dash_sdk::dpp::data_contract::accessors::v0::DataContractV0Getters;
use dash_sdk::dpp::voting::vote_polls::contested_document_resource_vote_poll::ContestedDocumentResourceVotePoll;
use dash_sdk::dpp::voting::vote_polls::VotePoll;
use dash_sdk::drive::query::VotePollsByEndDateDriveQuery;
use dash_sdk::platform::FetchMany;
use std::collections::BTreeMap;
use std::sync::Arc;

use dash_sdk::Sdk;

impl AppContext {
    pub(super) async fn query_dpns_ending_times(self: &Arc<Self>, sdk: Sdk) -> Result<(), String> {
        let now: DateTime<Utc> = Utc::now();
        let start_time_dt = now - Duration::weeks(2);
        let end_time_dt = now + Duration::weeks(2);
        let start_time = Some((start_time_dt.timestamp_millis() as u64, true));
        let end_time = Some((end_time_dt.timestamp_millis() as u64, true));

        let end_time_query = VotePollsByEndDateDriveQuery {
            start_time,
            end_time,
            limit: None,
            offset: None,
            order_ascending: true,
        };

        let mut contests_end_times = BTreeMap::new();

        for (timestamp, vote_poll) in VotePoll::fetch_many(&sdk, end_time_query)
            .await
            .map_err(|e| format!("error querying vote poll end times: {}", e))?
        {
            let contests = vote_poll.into_iter().filter_map(|vote_poll| {
                let VotePoll::ContestedDocumentResourceVotePoll(
                    ContestedDocumentResourceVotePoll {
                        contract_id,
                        document_type_name,
                        index_name,
                        index_values,
                    },
                ) = vote_poll;
                if contract_id != self.dpns_contract.id() {
                    return None;
                }
                if document_type_name != "domain" {
                    return None;
                }
                if index_name != "parentNameAndLabel" {
                    return None;
                }
                if index_values.len() != 2 {
                    return None;
                }
                index_values
                    .get(1)
                    .and_then(|a| a.to_str().ok().map(|a| (a.to_string(), timestamp)))
            });

            contests_end_times.extend(contests);
        }

        self.db
            .update_ending_time(contests_end_times, self)
            .map_err(|e| format!("error updating ending time: {}", e))
    }
}
